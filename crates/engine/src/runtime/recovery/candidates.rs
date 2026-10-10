use super::*;
use std::collections::{BTreeMap, BTreeSet};

impl Worker {
    pub(super) fn activate_next_parity(
        &self,
        job: &JobRow,
        parts: &[SegmentRow],
        inspection: Option<&postproc::Par2Inspection>,
        force: bool,
    ) -> Result<bool, String> {
        let rows = self
            .catalog
            .recovery_candidates(job.id)
            .map_err(|e| e.to_string())?;
        let mut grouped: BTreeMap<&str, Vec<&store::RecoveryCandidate>> = BTreeMap::new();
        for row in &rows {
            grouped.entry(&row.segment.filename).or_default().push(row);
        }
        let mut files = Vec::new();
        for (name, group) in grouped {
            let mut candidates = BTreeMap::new();
            for row in &group {
                if row.state == "deferred"
                    || (row.state == "requested"
                        && parts.iter().any(|part| {
                            part.message_id == row.segment.message_id && part.state == "written"
                        }))
                {
                    candidates.insert(row.segment.part, *row);
                }
            }
            let total = group[0].expected_parts;
            if total <= 0
                || candidates.len() as i64 != total
                || !group.iter().all(|row| row.expected_parts == total)
                || !candidates
                    .keys()
                    .enumerate()
                    .all(|(index, part)| *part == index as i64 + 1)
                || !candidates.values().any(|row| row.state == "deferred")
            {
                continue;
            }
            files.push((
                name,
                group[0].recovery_blocks,
                candidates.values().map(|row| row.id).collect::<Vec<_>>(),
            ));
        }
        let used = local_ranges(
            Path::new(&job.dest_dir),
            inspection.map(|info| info.set_id.as_str()),
        );
        files.retain(|(name, _, _)| {
            force
                || names::volume_range(name).is_none_or(|range| {
                    !used
                        .iter()
                        .any(|used| used.0 <= range.0 && used.1 >= range.1)
                })
        });
        let deficit = inspection
            .map(|info| info.blocks_needed.saturating_sub(info.blocks_available))
            .unwrap_or(1)
            .max(1) as i64;
        files.sort_by_key(|(name, blocks, _)| {
            // Fetch a small index before any volume. Then prefer the smallest
            // complete volume satisfying the deficit, or the largest smaller one.
            match blocks {
                None => (0, 0, (*name).to_string()),
                Some(count) if *count >= deficit => (1, *count, (*name).to_string()),
                Some(count) => (2, -*count, (*name).to_string()),
            }
        });
        for (name, _, ids) in files {
            let appended = self
                .catalog
                .activate_recovery_candidates(job.id, &ids)
                .map_err(|e| e.to_string())?;
            if appended == 0 {
                self.catalog
                    .exhaust_recovery_candidates(job.id, &ids)
                    .map_err(|e| e.to_string())?;
                continue;
            }
            self.catalog
                .set_job_state(job.id, "recovery_fetching", None)
                .map_err(|e| e.to_string())?;
            self.notice(format!("job {} fetching recovery file {name}", job.id));
            self.emit_jobs();
            return Ok(true);
        }
        Ok(false)
    }
}

fn local_ranges(dir: &Path, pinned: Option<&str>) -> BTreeSet<(u64, u64)> {
    let Some(pinned) = pinned else {
        return BTreeSet::new();
    };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let range = names::volume_range(entry.file_name().to_str()?)?;
            let set = rust_par2::parse(&entry.path()).ok()?;
            let id: String = set
                .recovery_set_id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            (id == pinned
                && u64::from(set.recovery_block_count) == range.1 - range.0
                && postproc::volume_covers_range(&entry.path(), pinned, set.slice_size, range))
            .then_some(range)
        })
        .collect()
}
