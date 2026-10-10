//! Replace overlapping verified volume layouts without duplicate recovery exponents.
use super::*;

impl Worker {
    pub(super) fn retire_recovery_overlaps(
        &self,
        job: &JobRow,
        parts: &[SegmentRow],
        pinned: &str,
    ) -> Result<(), String> {
        let rows = self
            .catalog
            .recovery_candidates(job.id)
            .map_err(|error| error.to_string())?;
        let dir = Path::new(&job.dest_dir);
        if !std::fs::symlink_metadata(dir)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_dir()
        {
            return Err("repair target must be a regular download directory".into());
        }
        let mut files = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(range) = names::volume_range(&name) else {
                continue;
            };
            if !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_file()
            {
                return Err("PAR2 recovery files must be regular local files".into());
            }
            let Ok(set) = rust_par2::parse(&entry.path()) else {
                continue;
            };
            let id: String = set
                .recovery_set_id
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            if id != pinned {
                continue;
            }
            let complete = u64::from(set.recovery_block_count) == range.1 - range.0
                && postproc::volume_covers_range(&entry.path(), pinned, set.slice_size, range);
            let rank = parts
                .iter()
                .filter(|part| {
                    part.filename == name
                        && part.state == "written"
                        && rows.iter().any(|row| {
                            row.segment.message_id == part.message_id
                                && row.segment.filename == name
                                && row.state == "requested"
                        })
                })
                .map(|part| part.id)
                .max()
                .unwrap_or(0);
            files.push((if complete { rank } else { 0 }, name, range, set.slice_size));
        }
        // The most recently requested valid volume replaces prior overlapping
        // layouts. An advertised filename alone never retires local data.
        files.sort_by_key(|(rank, name, _, _)| (std::cmp::Reverse(*rank), name.clone()));
        let mut kept: Vec<(i64, (u64, u64))> = Vec::new();
        for (rank, name, range, slice_size) in files {
            if kept.iter().any(|(prior_rank, prior)| {
                *prior_rank > 0
                    && postproc::volume_overlaps_range(&dir.join(&name), pinned, slice_size, *prior)
            }) {
                let retired = dir.join(".recovery-retired");
                if std::fs::symlink_metadata(&retired).is_ok_and(|meta| !meta.file_type().is_dir())
                {
                    return Err("recovery retirement target must be a regular directory".into());
                }
                std::fs::create_dir_all(&retired).map_err(|error| error.to_string())?;
                let target = retired.join(&name);
                if target.exists() {
                    return Err("a retired recovery volume already exists".into());
                }
                std::fs::rename(dir.join(&name), target).map_err(|error| error.to_string())?;
                self.notice(format!(
                    "job {} retained replaced recovery file {name} in .recovery-retired",
                    job.id
                ));
            } else {
                kept.push((rank, range));
            }
        }
        Ok(())
    }
}
