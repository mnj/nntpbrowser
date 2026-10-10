//! Offset-free UU article fragments remain separate until all parts are received.

use super::Worker;
use crate::decode;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use store::{JobRow, SegmentRow};

fn staged(job: &JobRow, segment: &SegmentRow) -> PathBuf {
    stage_dir(job).join(format!("{}.body", segment.id))
}

fn stage_dir(job: &JobRow) -> PathBuf {
    Path::new(&job.dest_dir).join(format!(".uu-parts-{}", job.id))
}

impl Worker {
    pub(super) fn stage_uu_segment(
        &self,
        job: &JobRow,
        segment: &SegmentRow,
        server: i64,
        body: &[u8],
    ) -> Result<(), String> {
        let path = staged(job, segment);
        if std::fs::symlink_metadata(path.parent().unwrap())
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err("uuencode staging directory is a symlink".into());
        }
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|error| error.to_string())?;
        std::fs::write(stage_dir(job).join(".owner"), job.id.to_string())
            .map_err(|error| error.to_string())?;
        let pending = path.with_extension("pending");
        std::fs::write(&pending, body).map_err(|error| error.to_string())?;
        std::fs::rename(pending, path).map_err(|error| error.to_string())?;
        self.catalog
            .set_segment_state(segment.id, "written", Some(server))
            .map_err(|error| error.to_string())
    }

    pub(super) async fn finish_uu_downloads(
        &self,
        job: &JobRow,
        segments: &[SegmentRow],
    ) -> Result<(), String> {
        let mut files: BTreeMap<&str, Vec<&SegmentRow>> = BTreeMap::new();
        for segment in segments {
            files.entry(&segment.filename).or_default().push(segment);
        }
        let mut total = 0i64;
        let mut assembled_any = false;
        for (name, mut parts) in files {
            if !parts.iter().any(|part| staged(job, part).exists()) {
                for part in parts.iter().filter(|part| part.state == "written") {
                    total = total.saturating_add(part.end.saturating_sub(part.begin).max(0));
                }
                continue;
            }
            parts.sort_by_key(|part| part.part);
            if !parts.iter().enumerate().all(|(index, part)| {
                part.part == index as i64 + 1
                    && part.state == "written"
                    && staged(job, part).is_file()
            }) {
                return Err(format!(
                    "uuencode {name}: multipart article sequence is incomplete"
                ));
            }
            if let Some(post_id) = job.post_id {
                let expected = self
                    .catalog
                    .files_for_post(post_id)
                    .map_err(|error| error.to_string())?;
                if expected
                    .iter()
                    .find(|file| file.name == name)
                    .is_some_and(|file| file.expected_segments != parts.len() as i64)
                {
                    return Err(format!("uuencode {name}: missing expected article parts"));
                }
            }
            let paths = parts
                .iter()
                .map(|part| staged(job, part))
                .collect::<Vec<_>>();
            let path = Path::new(&job.dest_dir).join(name);
            // Decoding is streamed; the 8 GiB limit is independent of RAM usage.
            let expected_name = name.to_owned();
            let size = tokio::task::spawn_blocking(move || {
                decode::write_uuencode_files_limited(
                    &paths,
                    &path,
                    &expected_name,
                    8 * 1024 * 1024 * 1024,
                )
            })
            .await
            .map_err(|error| format!("uuencode worker: {error}"))?
            .map_err(|error| format!("{name}: {error}"))?;
            total = total.saturating_add(size as i64);
            assembled_any = true;
            for (index, part) in parts.iter().enumerate() {
                // Persist exactly one file-size span, making progress stable
                // across retry even after the final output has been published.
                self.catalog
                    .set_segment_bounds(
                        part.id,
                        if index == 0 { 0 } else { size as i64 },
                        size as i64,
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        if assembled_any {
            self.catalog
                .set_job_progress(job.id, total, total)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn cleanup_uu_stage(&self, job: &JobRow) {
        if self
            .catalog
            .job_by_id(job.id)
            .ok()
            .flatten()
            .is_none_or(|current| current.state != "done" || current.dest_dir != job.dest_dir)
        {
            return;
        }
        let dir = stage_dir(job);
        if std::fs::symlink_metadata(&dir)
            .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
            && std::fs::read_to_string(dir.join(".owner"))
                .is_ok_and(|owner| owner == job.id.to_string())
        {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    pub(super) fn cleanup_completed_uu_stages(&self) {
        if let Ok(jobs) = self.catalog.list_jobs() {
            for job in jobs.iter().filter(|job| job.state == "done") {
                self.cleanup_uu_stage(job);
            }
        }
    }
}
