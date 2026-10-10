//! Durable, on-demand parity recovery using the normal bounded transfer queue.

use super::Worker;
use crate::postproc;
use std::{collections::HashMap, path::Path};
use store::{JobRecoveryContext, JobRow, SegmentRow};

mod candidates;
mod controls;
mod initialize;
mod names;
mod search;
mod volumes;
pub(super) use initialize::split_parity;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum RecoveryStep {
    Ready,
    Scheduled,
    Halted,
}

impl Worker {
    pub(super) async fn prepare_recovery(
        &mut self,
        job: &JobRow,
        parts: &[SegmentRow],
        required: &[String],
        force: bool,
    ) -> Result<RecoveryStep, String> {
        self.initialize_recovery(job)?;
        let mut context = self
            .catalog
            .recovery_context(job.id)
            .map_err(|e| e.to_string())?
            .ok_or("recovery context was not saved")?;
        if let Some(pinned) = context.set_id.as_deref() {
            self.retire_recovery_overlaps(job, parts, pinned)?;
        }
        let generation = self.transfer_generation.get(&job.id).copied().unwrap_or(0);
        let dir = Path::new(&job.dest_dir);
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let inspection = {
            let dir = dir.to_owned();
            let required = required.to_vec();
            let pinned = context.set_id.clone();
            let future = tokio::task::spawn_blocking(move || {
                postproc::inspect_dir_checked(&dir, &required, pinned.as_deref())
            });
            let Some(result) = self.recovery_blocking(job.id, generation, future).await else {
                return Ok(RecoveryStep::Halted);
            };
            result.map_err(|e| format!("PAR2 inspection worker: {e}"))?
        };
        let mut diagnostic = None;
        let inspection = match inspection {
            Ok(info) => info,
            Err(error) => {
                let text = error.to_string();
                if text.contains("regular") || text.contains("filename") {
                    return Err(text);
                }
                diagnostic = Some(text);
                None
            }
        };
        if let Some(info) = &inspection {
            if context.set_id.is_none() {
                context.set_id = Some(info.set_id.clone());
                self.catalog
                    .set_recovery_context(job.id, &context)
                    .map_err(|e| e.to_string())?;
            }
            if info.clean || (!force && info.blocks_needed <= info.blocks_available) {
                return Ok(RecoveryStep::Ready);
            }
            self.notice(format!(
                "job {} needs {} recovery blocks; {} are available",
                job.id, info.blocks_needed, info.blocks_available
            ));
        } else if !force
            && !self
                .download_verification_requirements(job, parts)?
                .requires_par2
        {
            // A structurally complete payload with no associated parity needs
            // no speculative header search.
            if diagnostic.is_none() {
                return Ok(RecoveryStep::Ready);
            }
        }
        if self.activate_next_parity(job, parts, inspection.as_ref(), force)? {
            return Ok(RecoveryStep::Scheduled);
        }
        if !context.discovery_exhausted && context.server_id.is_some() && context.group_id.is_some()
        {
            let step = self
                .discover_job_recovery(job, parts, &mut context, inspection.as_ref(), generation)
                .await?;
            if step != RecoveryStep::Ready {
                return Ok(step);
            }
        }
        if let Some(text) = diagnostic {
            return Err(text);
        }
        Ok(RecoveryStep::Ready)
    }
}
