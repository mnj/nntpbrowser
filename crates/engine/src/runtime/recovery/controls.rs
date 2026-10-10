use super::*;
use std::{future::Future, time::Duration};

impl Worker {
    pub(in crate::runtime) fn recovery_current(&self, job: i64, generation: u64) -> bool {
        self.transfer_generation.get(&job).copied().unwrap_or(0) == generation
            && !self.paused.contains(&job)
            && !self.cancelled.contains(&job)
            && self
                .catalog
                .job_by_id(job)
                .ok()
                .flatten()
                .is_some_and(|row| !matches!(row.state.as_str(), "paused" | "cancelled"))
    }

    pub(super) async fn recovery_network<F: Future>(
        &mut self,
        job: i64,
        generation: u64,
        future: F,
    ) -> Option<F::Output> {
        tokio::pin!(future);
        loop {
            tokio::select! {
                result = &mut future => { self.poll_controls(); return self.recovery_current(job, generation).then_some(result); },
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    self.poll_controls();
                    if !self.recovery_current(job, generation) { return None; }
                }
            }
        }
    }

    /// Blocking repair writes cannot be aborted. Keep their handle until the
    /// operation settles, while accepting controls and fencing its next stage.
    pub(in crate::runtime) async fn recovery_blocking<F: Future>(
        &mut self,
        job: i64,
        generation: u64,
        future: F,
    ) -> Option<F::Output> {
        tokio::pin!(future);
        let mut stale = false;
        loop {
            tokio::select! {
                result = &mut future => {
                    self.poll_controls();
                    return (!stale && self.recovery_current(job, generation)).then_some(result);
                },
                _ = tokio::time::sleep(Duration::from_millis(25)) => {
                    self.poll_controls();
                    stale |= !self.recovery_current(job, generation);
                }
            }
        }
    }
}
