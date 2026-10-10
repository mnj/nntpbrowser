//! Byte transfer progress and post-processing are separate stages.
use super::super::*;

pub(crate) struct JobProgress {
    pub(crate) fraction: f32,
    pub(crate) animate: bool,
    pub(crate) color: Color32,
    pub(crate) stage: &'static str,
    pub(crate) detail: String,
}

impl JobProgress {
    pub(crate) fn new(job: &JobRow) -> Self {
        let stage = match job.state.as_str() {
            "fetching" => "Downloading",
            "recovery_headers" => "Finding missing parts…",
            "recovery_fetching" => "Downloading repair data",
            "decoding" => "Decoding files…",
            "repair" => "Verifying / repairing…",
            "extract" => "Extracting files…",
            "paused" => "Paused",
            "failed" => "Failed",
            "done" => "Completed",
            "cancelled" => "Cancelled",
            _ => "Waiting in queue",
        };
        let processing = matches!(
            job.state.as_str(),
            "decoding" | "repair" | "extract" | "recovery_headers"
        );
        // Recovery may discover volumes absent from the original size estimate.
        let recovery_transfer = job.state == "recovery_fetching";
        let ratio = if processing || recovery_transfer {
            None
        } else if job.state == "done" {
            Some(1.0)
        } else {
            byte_ratio(job.bytes_done, job.bytes_total)
        };
        let detail = if job.state == "recovery_headers" {
            "Looking for matching article and repair headers".into()
        } else if recovery_transfer {
            "Fetching additional parts as needed".into()
        } else if processing {
            "Files downloaded · processing".into()
        } else if let Some(ratio) = ratio {
            format!("{:.0}% transferred", ratio * 100.0)
        } else {
            "Size unknown".into()
        };
        let color = match job.state.as_str() {
            "failed" | "cancelled" => theme::DANGER,
            "paused" | "repair" | "extract" | "recovery_headers" | "recovery_fetching" => {
                theme::WARN
            }
            _ => theme::ACCENT,
        };
        Self {
            fraction: ratio.unwrap_or(0.0),
            animate: processing
                || matches!(job.state.as_str(), "fetching" | "recovery_fetching")
                || (job.state == "queued" && ratio.is_none()),
            color,
            stage,
            detail,
        }
    }
}

pub(crate) struct AggregateProgress {
    pub(crate) count: usize,
    pub(crate) active: usize,
    pub(crate) fraction: Option<f32>,
    pub(crate) animate: bool,
    pub(crate) color: Color32,
    pub(crate) detail: String,
    pub(crate) stages: String,
    pub(crate) compact: String,
}

impl AggregateProgress {
    #[cfg(test)]
    pub(crate) fn new(jobs: &[JobRow]) -> Self {
        Self::from_jobs(jobs.iter())
    }

    pub(crate) fn active(jobs: &[JobRow]) -> Self {
        Self::from_jobs(jobs.iter().filter(|job| is_active(&job.state)))
    }

    fn from_jobs<'a>(jobs: impl Iterator<Item = &'a JobRow>) -> Self {
        let mut active = 0;
        let mut paused = 0;
        let mut failed = 0;
        let mut queued = 0;
        let mut fetching = 0;
        let mut recovery_headers = 0;
        let mut recovery_fetching = 0;
        let mut decoding = 0;
        let mut repair = 0;
        let mut extract = 0;
        let mut done_bytes = 0i64;
        let mut total_bytes = 0i64;
        let mut unknown = false;
        for job in jobs {
            match job.state.as_str() {
                "queued" => {
                    active += 1;
                    queued += 1;
                }
                "fetching" => {
                    active += 1;
                    fetching += 1;
                }
                "recovery_headers" => {
                    active += 1;
                    recovery_headers += 1;
                }
                "recovery_fetching" => {
                    active += 1;
                    recovery_fetching += 1;
                }
                "decoding" => {
                    active += 1;
                    decoding += 1;
                }
                "repair" => {
                    active += 1;
                    repair += 1;
                }
                "extract" => {
                    active += 1;
                    extract += 1;
                }
                "paused" => paused += 1,
                "failed" => failed += 1,
                _ => continue,
            }
            if job.bytes_total > 0 {
                total_bytes = total_bytes.saturating_add(job.bytes_total);
                done_bytes = done_bytes.saturating_add(job.bytes_done.clamp(0, job.bytes_total));
            } else {
                unknown = true;
                done_bytes = done_bytes.saturating_add(job.bytes_done.max(0));
            }
        }
        let count = active + paused + failed;
        let processing = decoding + repair + extract + recovery_headers;
        let only_processing =
            processing > 0 && queued == 0 && fetching == 0 && recovery_fetching == 0;
        let fraction = if unknown || only_processing || recovery_fetching > 0 {
            None
        } else {
            byte_ratio(done_bytes, total_bytes)
        };
        let mut stages = Vec::new();
        for (count, label) in [
            (fetching, "downloading"),
            (recovery_headers, "finding missing parts"),
            (recovery_fetching, "fetching repair data"),
            (queued, "queued"),
            (decoding, "decoding"),
            (repair, "verifying / repairing"),
            (extract, "extracting"),
            (paused, "paused"),
            (failed, "failed"),
        ] {
            if count > 0 {
                stages.push(format!("{count} {label}"));
            }
        }
        let detail = if only_processing && recovery_headers > 0 {
            format!(
                "{} transferred · finding missing parts",
                format_bytes(done_bytes)
            )
        } else if only_processing {
            format!(
                "{} transferred · processing files",
                format_bytes(done_bytes)
            )
        } else if recovery_fetching > 0 {
            format!(
                "{} transferred · fetching additional parts",
                format_bytes(done_bytes)
            )
        } else if unknown {
            format!(
                "{} transferred · total size unknown",
                format_bytes(done_bytes)
            )
        } else {
            format!(
                "{} / {} transferred",
                format_bytes(done_bytes),
                format_bytes(total_bytes)
            )
        };
        let compact = if let Some(fraction) = fraction {
            format!("{:.0}%", fraction * 100.0)
        } else if only_processing {
            "Processing".into()
        } else if active > 0 {
            "Working".into()
        } else if failed > 0 {
            "Failed".into()
        } else {
            "Paused".into()
        };
        let color = if failed > 0 {
            theme::DANGER
        } else if (active == 0 && paused > 0) || only_processing {
            theme::WARN
        } else {
            theme::ACCENT
        };
        Self {
            count,
            active,
            fraction,
            animate: active > 0,
            color,
            detail,
            stages: stages.join(" · "),
            compact,
        }
    }
}

fn byte_ratio(done: i64, total: i64) -> Option<f32> {
    (total > 0).then(|| (done.clamp(0, total) as f64 / total as f64) as f32)
}

#[cfg(test)]
mod tests;
