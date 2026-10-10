use super::*;
use crate::runtime::recovery_discovery::{self, Discovery, DiscoveryPlan, OVERVIEW_BATCH};

impl Worker {
    pub(super) async fn discover_job_recovery(
        &mut self,
        job: &JobRow,
        parts: &[SegmentRow],
        context: &mut JobRecoveryContext,
        inspection: Option<&postproc::Par2Inspection>,
        generation: u64,
    ) -> Result<RecoveryStep, String> {
        let Some(server) = context
            .server_id
            .and_then(|id| self.server(id))
            .filter(|server| server.enabled)
        else {
            return Ok(RecoveryStep::Ready);
        };
        let mut payload_names: Vec<_> = parts
            .iter()
            .filter(|part| store::classify(&part.filename) != store::FileKind::Par2)
            .map(|part| part.filename.clone())
            .collect();
        if let Some(post) = job.post_id {
            payload_names.extend(
                self.catalog
                    .files_for_post(post)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .filter(|file| file.kind != store::FileKind::Par2)
                    .map(|file| file.name),
            );
        }
        if let Some(info) = inspection {
            payload_names.extend(info.protected_names.clone());
        }
        payload_names.sort();
        payload_names.dedup();
        let candidates = self
            .catalog
            .recovery_candidates(job.id)
            .map_err(|e| e.to_string())?;
        let mut known = parts.to_vec();
        for row in &candidates {
            known.push(SegmentRow {
                id: -row.id,
                job_id: job.id,
                filename: row.segment.filename.clone(),
                part: row.segment.part,
                message_id: row.segment.message_id.clone(),
                begin: row.segment.begin,
                end: row.segment.end,
                article: row.segment.article,
                state: row.state.clone(),
                server_id: None,
            });
        }
        let mut discovery =
            Discovery::new(&context.group_key, &context.poster, &payload_names, &known)?
                .with_posted_at(context.posted_at);
        discovery.seed_candidates(
            &candidates
                .iter()
                .map(|row| {
                    (
                        row.segment.clone(),
                        u32::try_from(row.expected_parts).unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>(),
        )?;
        let anchors: Vec<_> = known
            .iter()
            .filter_map(|part| part.article.and_then(|number| u64::try_from(number).ok()))
            .collect();
        if anchors.is_empty() {
            context.discovery_exhausted = true;
            self.catalog
                .set_recovery_context(job.id, context)
                .map_err(|e| e.to_string())?;
            return Ok(RecoveryStep::Ready);
        }
        self.catalog
            .set_job_state(job.id, "recovery_headers", None)
            .map_err(|e| e.to_string())?;
        self.emit_jobs();
        let stored = self
            .headers
            .stored_ranges(server.id, &context.group)
            .map_err(|e| e.to_string())?;
        let cached_plan =
            DiscoveryPlan::new(&anchors, 1, u64::MAX, &stored, &context.discovery_ranges);
        for (low, high) in cached_plan.cached_windows {
            for &(stored_low, stored_high) in &stored {
                let mut first = low.max(stored_low);
                let last = high.min(stored_high);
                while first <= last {
                    self.poll_controls();
                    if !self.recovery_current(job.id, generation) {
                        return Ok(RecoveryStep::Halted);
                    }
                    let end = first.saturating_add(OVERVIEW_BATCH - 1).min(last);
                    recovery_discovery::scan_cached(
                        &self.headers,
                        server.id,
                        &context.group,
                        first,
                        end,
                        &mut discovery,
                    )?;
                    self.poll_controls();
                    if !self.recovery_current(job.id, generation) {
                        return Ok(RecoveryStep::Halted);
                    }
                    if self.persist_discovery(job, &discovery)?
                        || self.activate_next_parity(job, parts, inspection, false)?
                    {
                        return Ok(RecoveryStep::Scheduled);
                    }
                    if end == u64::MAX {
                        break;
                    }
                    first = end + 1;
                }
            }
        }
        self.ensure_pool(&server).await;
        let pool = self
            .pools
            .get(&server.id)
            .cloned()
            .ok_or("recovery server has no pool")?;
        let Some(status) = self
            .recovery_network(job.id, generation, pool.group(&context.group))
            .await
        else {
            return Ok(RecoveryStep::Halted);
        };
        let status = status.map_err(|e| format!("recovery group: {e}"))?;
        let plan = DiscoveryPlan::new(
            &anchors,
            status.low,
            status.high,
            &stored,
            &context.discovery_ranges,
        );
        for (first, last) in plan.windows {
            let Some(records) = self
                .recovery_network(
                    job.id,
                    generation,
                    recovery_discovery::overview_batch(&pool, first, last),
                )
                .await
            else {
                return Ok(RecoveryStep::Halted);
            };
            for record in records? {
                discovery.ingest(record);
            }
            let appended = self.persist_discovery(job, &discovery)?;
            self.catalog
                .mark_recovery_scanned(job.id, first, last)
                .map_err(|e| e.to_string())?;
            context.discovery_ranges = self
                .catalog
                .recovery_scanned_ranges(job.id)
                .map_err(|e| e.to_string())?;
            if appended || self.activate_next_parity(job, parts, inspection, false)? {
                return Ok(RecoveryStep::Scheduled);
            }
            self.drive_previews().await;
        }
        context.discovery_exhausted = true;
        self.catalog
            .set_recovery_context(job.id, context)
            .map_err(|e| e.to_string())?;
        Ok(RecoveryStep::Ready)
    }

    fn persist_discovery(&self, job: &JobRow, discovery: &Discovery) -> Result<bool, String> {
        let candidates = discovery
            .partial_candidates()
            .into_iter()
            .flat_map(|file| {
                file.segments.into_iter().map(move |segment| {
                    (
                        segment,
                        file.recovery_blocks.map(i64::from),
                        i64::from(file.expected_parts),
                    )
                })
            })
            .collect::<Vec<_>>();
        self.catalog
            .add_recovery_candidates(job.id, &candidates)
            .map_err(|e| e.to_string())?;
        let added = self
            .catalog
            .append_job_segments(job.id, &discovery.missing_payload_segments())
            .map_err(|e| e.to_string())?;
        if added > 0 {
            self.catalog
                .set_job_state(job.id, "recovery_fetching", None)
                .map_err(|e| e.to_string())?;
            self.emit_jobs();
        }
        Ok(added > 0)
    }
}
