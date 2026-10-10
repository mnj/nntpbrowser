//! Concurrent FIFO pipeline windows; only this worker mutates files/catalog.

use super::Worker;
use crate::decode;
use nntp::{ArticleRef, Pool};
use std::{path::Path, sync::Arc, time::Duration};
use store::{JobRow, SegmentRow};

struct Transfer {
    segment: SegmentRow,
    generation: u64,
    limit: usize,
    cached: Option<Vec<u8>>,
}

struct TransferResult {
    segment: SegmentRow,
    generation: u64,
    result: Result<(i64, Vec<u8>), nntp::Error>,
}

pub(super) struct WindowResult(Vec<TransferResult>);

impl Worker {
    pub(super) async fn drive_downloads(&mut self) -> bool {
        self.poll_controls();
        let mut jobs = match self.catalog.list_jobs() {
            Ok(jobs) => jobs,
            Err(_) => return false,
        };
        jobs.sort_by_key(|job| (self.media_job_limit(job).is_some(), job.id));
        let mut servers = self.catalog.list_servers().unwrap_or_default();
        servers.retain(|server| server.enabled);
        servers.sort_by_key(|server| (server.fill, server.priority));
        for server in &servers {
            self.ensure_pool(server).await;
        }
        let capacity = servers
            .iter()
            .fold(0usize, |sum, server| {
                sum.saturating_add(server.max_connections.clamp(1, 128) as usize)
            })
            .clamp(1, 64);
        let pools = servers
            .iter()
            .filter_map(|server| self.pools.get(&server.id).cloned())
            .collect::<Vec<_>>();
        let mut worked = false;
        for job in &jobs {
            if matches!(
                job.state.as_str(),
                "done" | "cancelled" | "failed" | "paused"
            ) {
                continue;
            }
            if let Err(error) = self.initialize_recovery(job) {
                self.fail_job(job.id, error);
                continue;
            }
            let parts = match self.catalog.segments_for_job(job.id) {
                Ok(parts) => parts,
                Err(error) => {
                    self.fail_job(job.id, error.to_string());
                    continue;
                }
            };
            if parts.iter().any(|part| {
                part.filename.is_empty()
                    || matches!(part.filename.as_str(), "." | "..")
                    || part.filename.contains(['/', '\\', ':'])
            }) {
                self.fail_job(job.id, "download filename is invalid".into());
                continue;
            }
            let has_flights = self.in_flight.values().any(|(job_id, _)| *job_id == job.id);
            let mut pending = parts
                .iter()
                .filter(|part| part.state == "pending" && !self.in_flight.contains_key(&part.id))
                .cloned()
                .collect::<Vec<_>>();
            if pending.is_empty() {
                if !has_flights {
                    self.finish_job(job, &parts).await;
                    worked = true;
                }
                continue;
            }
            let media_limit = self.media_job_limit(job);
            let available = capacity.saturating_sub(self.transfers.len());
            if available == 0 {
                continue;
            }
            let auto_flights = self
                .in_flight
                .values()
                .filter(|(id, _)| {
                    jobs.iter()
                        .any(|job| job.id == *id && self.media_job_limit(job).is_some())
                })
                .count();
            if media_limit.is_some() {
                pending.truncate(4usize.saturating_sub(auto_flights));
            }
            if pending.is_empty() {
                continue;
            }
            let configured = pools.first().map(|pool| pool.pipeline_depth()).unwrap_or(1);
            let estimate = if job.bytes_total > 0 {
                (job.bytes_total as usize)
                    .div_ceil(parts.len().max(1))
                    .max(1)
            } else {
                1024 * 1024
            };
            let budget_depth = (8 * 1024 * 1024 / estimate).max(1);
            let depth = if media_limit.is_some() {
                1
            } else {
                configured
                    .min(budget_depth)
                    .min(pending.len().div_ceil(available))
                    .max(1)
            };
            for window in pending.chunks(depth) {
                if self.transfers.len() >= capacity {
                    break;
                }
                let generation = *self.transfer_generation.get(&job.id).unwrap_or(&0);
                let mut transfers = Vec::with_capacity(window.len());
                for segment in window {
                    let limit = media_limit
                        .map(|cap| (cap * 2 + 65_536) as usize)
                        .unwrap_or(usize::MAX);
                    let cached = media_limit.and(job.post_id).and_then(|post_id| {
                        self.cached_article_body(post_id, &segment.message_id, limit)
                    });
                    if self
                        .catalog
                        .set_segment_state(segment.id, "fetching", None)
                        .is_err()
                    {
                        continue;
                    }
                    self.in_flight.insert(segment.id, (job.id, generation));
                    transfers.push(Transfer {
                        segment: segment.clone(),
                        generation,
                        limit,
                        cached,
                    });
                }
                if !transfers.is_empty() {
                    let state = if job.state == "recovery_fetching" {
                        "recovery_fetching"
                    } else {
                        "fetching"
                    };
                    let _ = self.catalog.set_job_state(job.id, state, None);
                    self.transfers.spawn(fetch_window(pools.clone(), transfers));
                    worked = true;
                }
            }
        }
        if worked {
            self.emit_jobs();
        }
        if self.transfers.is_empty() {
            return worked;
        }
        let completed = tokio::select! {
            result=self.transfers.join_next()=>result,
            _=tokio::time::sleep(Duration::from_millis(40))=>return true,
        };
        self.poll_controls();
        match completed {
            Some(Ok(window)) => {
                for transfer in window.0 {
                    self.apply_transfer(transfer);
                }
            }
            Some(Err(error)) => {
                let flights = std::mem::take(&mut self.in_flight);
                self.transfers.abort_all();
                for (segment, (job, _)) in flights {
                    let _ = self.catalog.set_segment_state(segment, "pending", None);
                    self.fail_job(job, format!("download worker: {error}"));
                }
            }
            None => {}
        }
        true
    }

    fn apply_transfer(&mut self, transfer: TransferResult) {
        let segment = transfer.segment;
        self.in_flight.remove(&segment.id);
        let Some(job) = self.catalog.job_by_id(segment.job_id).ok().flatten() else {
            return;
        };
        if self.transfer_generation.get(&job.id).copied().unwrap_or(0) != transfer.generation
            || matches!(job.state.as_str(), "cancelled" | "paused" | "failed")
        {
            let _ = self.catalog.set_segment_state(segment.id, "pending", None);
            self.emit_jobs();
            return;
        }
        if let Ok((server, bytes)) = &transfer.result {
            if decode::is_uuencode_fragment(bytes)
                && self.catalog.segments_for_job(job.id).is_ok_and(|parts| {
                    parts
                        .iter()
                        .filter(|part| part.filename == segment.filename)
                        .count()
                        > 1
                })
            {
                if let Err(error) = self.stage_uu_segment(&job, &segment, *server, bytes) {
                    let _ = self.catalog.set_segment_state(segment.id, "failed", None);
                    self.notice(format!("download failed: {}: {error}", segment.filename));
                }
                self.emit_jobs();
                return;
            }
        }
        let result = match transfer.result {
            Ok((server, bytes)) => decode::decode_body(&bytes)
                .map(|part| (server, part))
                .map_err(|error| error.to_string()),
            Err(nntp::Error::Missing) => {
                let _ = self.catalog.set_segment_state(segment.id, "missing", None);
                if self.media_job_limit(&job).is_some() {
                    self.fail_job(job.id, "media article is missing".into());
                }
                self.emit_jobs();
                return;
            }
            Err(error) => Err(error.to_string()),
        };
        match result.and_then(|(server, part)| self.write_transfer(&job, &segment, server, &part)) {
            Ok(()) => {}
            Err(error) => {
                let _ = self.catalog.set_segment_state(segment.id, "failed", None);
                if self.media_job_limit(&job).is_some() {
                    self.fail_job(job.id, error);
                } else {
                    self.notice(format!("download failed: {}: {error}", segment.filename));
                }
            }
        }
        self.emit_jobs();
    }

    fn write_transfer(
        &self,
        job: &JobRow,
        segment: &SegmentRow,
        server: i64,
        part: &decode::Part,
    ) -> Result<(), String> {
        if let Some(cap) = self.media_job_limit(job) {
            self.validate_media_part(job, segment, part, cap)?;
        }
        let end = part
            .begin
            .checked_add(part.data.len() as u64)
            .ok_or("part offset overflow")?;
        let begin = i64::try_from(part.begin).map_err(|_| "part offset exceeds catalog range")?;
        let end = i64::try_from(end).map_err(|_| "part offset exceeds catalog range")?;
        decode::write_part(&Path::new(&job.dest_dir).join(&segment.filename), part)
            .map_err(|error| error.to_string())?;
        self.catalog
            .set_segment_bounds(segment.id, begin, end)
            .map_err(|error| error.to_string())?;
        self.catalog
            .set_segment_state(segment.id, "written", (server != 0).then_some(server))
            .map_err(|error| error.to_string())?;
        let done = job.bytes_done.saturating_add(part.data.len() as i64);
        self.catalog
            .set_job_progress(job.id, done, job.bytes_total.max(done))
            .map_err(|error| error.to_string())
    }
}

async fn fetch_window(pools: Vec<Arc<Pool>>, transfers: Vec<Transfer>) -> WindowResult {
    let mut pending = Vec::new();
    let mut output = Vec::with_capacity(transfers.len());
    for transfer in transfers {
        if let Some(bytes) = transfer.cached {
            output.push(TransferResult {
                segment: transfer.segment,
                generation: transfer.generation,
                result: Ok((0, bytes)),
            });
        } else {
            pending.push(transfer);
        }
    }
    for pool in pools {
        if pending.is_empty() {
            break;
        }
        let ids = pending
            .iter()
            .map(|transfer| ArticleRef::MessageId(transfer.segment.message_id.clone()))
            .collect::<Vec<_>>();
        let limits = pending
            .iter()
            .map(|transfer| transfer.limit)
            .collect::<Vec<_>>();
        let replies = match pool.body_many_limited(&ids, &limits).await {
            Ok(replies) => replies,
            Err(error) => {
                let message = error.to_string();
                for transfer in pending.drain(..) {
                    output.push(TransferResult {
                        segment: transfer.segment,
                        generation: transfer.generation,
                        result: Err(nntp::Error::Connect(message.clone())),
                    });
                }
                break;
            }
        };
        let mut missing = Vec::new();
        for (transfer, reply) in pending.drain(..).zip(replies) {
            match reply {
                Err(nntp::Error::Missing) => missing.push(transfer),
                result => output.push(TransferResult {
                    segment: transfer.segment,
                    generation: transfer.generation,
                    result: result.map(|bytes| (pool.server_id(), bytes)),
                }),
            }
        }
        pending = missing;
    }
    for transfer in pending {
        output.push(TransferResult {
            segment: transfer.segment,
            generation: transfer.generation,
            result: Err(nntp::Error::Missing),
        });
    }
    WindowResult(output)
}
