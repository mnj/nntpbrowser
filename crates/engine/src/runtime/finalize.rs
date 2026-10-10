//! Verification, on-demand recovery, and final publication of completed jobs.
use super::{recovery::RecoveryStep, Worker};
use crate::postproc::{self, ExtractReport, RepairReport};
use std::path::Path;
use store::{JobRow, SegmentRow};

impl Worker {
    pub(super) async fn finish_job(&mut self, job: &JobRow, segments: &[SegmentRow]) {
        if let Some(cap) = self.media_job_limit(job) {
            if let Err(error) = self.finish_media_job(job, segments, cap) {
                self.fail_job(job.id, error);
            }
            return;
        }
        let generation = self.transfer_generation.get(&job.id).copied().unwrap_or(0);
        if let Err(error) = self.initialize_recovery(job) {
            self.fail_job(job.id, error);
            return;
        }
        if segments
            .iter()
            .any(|part| matches!(part.state.as_str(), "pending" | "fetching"))
        {
            self.fail_job(job.id, "download has unfinished segments".into());
            return;
        }
        let assembled = self.finish_uu_downloads(job, segments).await;
        self.poll_controls();
        if !self.recovery_current(job.id, generation) {
            return;
        }
        if let Err(error) = assembled {
            self.fail_job(job.id, error);
            return;
        }
        let segments = match self.catalog.segments_for_job(job.id) {
            Ok(parts) => parts,
            Err(error) => {
                self.fail_job(job.id, error.to_string());
                return;
            }
        };
        let required = match self.download_verification_requirements(job, &segments) {
            Ok(required) => required,
            Err(error) => {
                self.fail_job(job.id, error);
                return;
            }
        };
        let has_payload = !required.payloads.is_empty();
        let dir = Path::new(&job.dest_dir);
        if let Err(error) = std::fs::create_dir_all(dir) {
            self.fail_job(job.id, error.to_string());
            return;
        }
        if has_payload {
            match self
                .prepare_recovery(job, &segments, &required.files, false)
                .await
            {
                Ok(RecoveryStep::Ready) => {}
                Ok(_) => return,
                Err(error) => {
                    self.fail_job(job.id, error);
                    return;
                }
            }
        }
        if segments.is_empty() && !has_payload {
            self.fail_job(job.id, "download has no article segments".into());
            return;
        }
        let pinned = self
            .catalog
            .recovery_context(job.id)
            .ok()
            .flatten()
            .and_then(|context| context.set_id);
        let _ = self.catalog.set_job_state(job.id, "repair", None);
        self.emit_jobs();
        let verify_names = required.files.clone();
        let repair = tokio::task::spawn_blocking({
            let parity_only = !has_payload;
            let dir = dir.to_owned();
            let pinned = pinned.clone();
            let names = verify_names.clone();
            move || {
                if parity_only {
                    return Ok((RepairReport::NoPar2, Vec::new()));
                }
                let report = postproc::repair_dir_pinned(&dir, &names, pinned.as_deref())?;
                let protected = if matches!(report, RepairReport::Clean | RepairReport::Repaired) {
                    postproc::inspect_dir_checked(&dir, &names, pinned.as_deref())?
                        .map(|info| info.protected_names)
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                Ok::<_, postproc::PostError>((report, protected))
            }
        });
        let Some(repair) = self.recovery_blocking(job.id, generation, repair).await else {
            return;
        };
        let mut protected = Vec::new();
        let error = match repair {
            Ok(Ok((RepairReport::Insufficient(text), _))) => Some(format!("repair: {text}")),
            Ok(Ok((RepairReport::NoPar2, _))) if required.requires_par2 => Some(format!(
                "incomplete or damaged files ({}) and no PAR2 recovery data is available",
                required.files.join(", ")
            )),
            Ok(Ok((_, names))) => {
                protected = names;
                None
            }
            Ok(Err(error)) => Some(error.to_string()),
            Err(error) => Some(format!("repair worker: {error}")),
        };
        if let Some(error) = error {
            if has_payload {
                match self
                    .prepare_recovery(job, &segments, &verify_names, true)
                    .await
                {
                    Ok(RecoveryStep::Scheduled | RecoveryStep::Halted) => return,
                    Ok(RecoveryStep::Ready) => {}
                    Err(recovery_error) => {
                        self.notice(format!("job {} recovery: {recovery_error}", job.id))
                    }
                }
            }
            if self.recovery_current(job.id, generation) {
                self.fail_job(job.id, error);
            }
            return;
        }
        if required.incomplete_parity {
            self.notice(
                "PAR2 verified the payload files; optional recovery files remain incomplete".into(),
            );
        }
        let mut completed_bytes = match self.downloaded_file_bytes(job, &segments) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.fail_job(job.id, error);
                return;
            }
        };
        let mut known: std::collections::BTreeSet<_> =
            segments.iter().map(|part| part.filename.clone()).collect();
        known.extend(required.payloads.iter().cloned());
        for name in protected.into_iter().filter(|name| !known.contains(name)) {
            completed_bytes = completed_bytes.saturating_add(
                std::fs::metadata(dir.join(name))
                    .ok()
                    .and_then(|meta| i64::try_from(meta.len()).ok())
                    .unwrap_or(0),
            );
        }
        if let Err(error) = self
            .catalog
            .set_job_progress(job.id, completed_bytes, completed_bytes)
        {
            self.fail_job(job.id, error.to_string());
            return;
        }
        let _ = self.catalog.set_job_state(job.id, "extract", None);
        let extracted = tokio::task::spawn_blocking({
            let dir = dir.to_owned();
            let password = job.password.clone();
            move || postproc::extract_archives(&dir, password.as_deref())
        });
        let Some(extracted) = self.recovery_blocking(job.id, generation, extracted).await else {
            return;
        };
        match extracted {
            Ok(Ok(ExtractReport::PackedRar)) => {
                self.notice("RAR archive left packed; install unrar or unar to extract it".into())
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                self.fail_job(job.id, error.to_string());
                return;
            }
            Err(error) => {
                self.fail_job(job.id, format!("extraction worker: {error}"));
                return;
            }
        }
        if !self.recovery_current(job.id, generation) {
            return;
        }
        self.write_thumbs(dir, job.post_id);
        if let Err(error) = self.catalog.set_job_state(job.id, "done", None) {
            self.notice(format!(
                "job {} failed to record completion: {error}",
                job.id
            ));
            return;
        }
        self.cleanup_uu_stage(job);
        if let Some(post_id) = job.post_id {
            self.emit_cached_media(post_id);
        }
        self.emit_jobs();
        self.notice(format!("job {} done", job.id));
    }
}
