//! Require known part coverage and actual byte coverage before accepting a download.

use super::Worker;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
use store::{JobRow, SegmentRow};

pub(super) struct DownloadRequirements {
    pub payloads: Vec<String>,
    pub files: Vec<String>,
    pub requires_par2: bool,
    pub incomplete_parity: bool,
}

impl Worker {
    /// The returned files need independent full-file PAR2 verification. Header
    /// totals are estimates; a matching verified PAR2 file can certify a complete
    /// download even when its cached overview part numbering is incomplete.
    pub(super) fn download_verification_requirements(
        &self,
        job: &JobRow,
        segments: &[SegmentRow],
    ) -> Result<DownloadRequirements, String> {
        let mut files: BTreeMap<String, (i64, Vec<&SegmentRow>)> = BTreeMap::new();
        if let Some(post_id) = job.post_id {
            for file in self
                .catalog
                .files_for_post(post_id)
                .map_err(|error| error.to_string())?
            {
                files.insert(file.name, (file.expected_segments, Vec::new()));
            }
        }
        for segment in segments {
            files
                .entry(segment.filename.clone())
                .or_default()
                .1
                .push(segment);
        }
        let deferred: BTreeSet<_> = self
            .catalog
            .recovery_candidates(job.id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|row| row.state == "deferred")
            .map(|row| row.segment.filename)
            .collect();
        let mut unresolved = Vec::new();
        let mut payloads = Vec::new();
        let mut unresolved_parity = Vec::new();
        let mut incomplete_parity = false;
        for (name, (expected, parts)) in files {
            let parity = store::classify(&name) == store::FileKind::Par2;
            if parity && parts.is_empty() && deferred.contains(&name) {
                continue;
            }
            if !parity {
                payloads.push(name.clone());
            }
            let distinct: BTreeSet<_> = parts.iter().map(|part| part.part).collect();
            let numbered = distinct
                .iter()
                .enumerate()
                .all(|(index, part)| *part == index as i64 + 1);
            if parts.is_empty()
                || parts.iter().any(|part| part.state != "written")
                || !numbered
                || (expected > 0 && distinct.len() as i64 != expected)
                || !covers_file(&Path::new(&job.dest_dir).join(&name), &parts)
            {
                if parity {
                    incomplete_parity = true;
                    unresolved_parity.push(name);
                } else {
                    unresolved.push(name);
                }
            }
        }
        // An unavailable auxiliary volume is tolerable only when one usable
        // local set certifies every payload, including apparently complete ones.
        let requires_par2 = incomplete_parity || !unresolved.is_empty();
        // A parity-only request means the recovery files themselves are the
        // requested output. Verifying unrelated payload cannot satisfy it.
        let auxiliary_parity = incomplete_parity && !payloads.is_empty();
        if !auxiliary_parity {
            unresolved.extend(unresolved_parity);
        }
        Ok(DownloadRequirements {
            files: if auxiliary_parity {
                payloads.clone()
            } else {
                unresolved
            },
            requires_par2,
            payloads,
            incomplete_parity: auxiliary_parity,
        })
    }

    pub(super) fn downloaded_file_bytes(
        &self,
        job: &JobRow,
        segments: &[SegmentRow],
    ) -> Result<i64, String> {
        let mut names: BTreeSet<_> = segments.iter().map(|part| part.filename.clone()).collect();
        if let Some(post_id) = job.post_id {
            names.extend(
                self.catalog
                    .files_for_post(post_id)
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .map(|file| file.name),
            );
        }
        Ok(names.into_iter().fold(0i64, |total, name| {
            let bytes = std::fs::metadata(Path::new(&job.dest_dir).join(name))
                .ok()
                .and_then(|meta| i64::try_from(meta.len()).ok())
                .unwrap_or(0);
            total.saturating_add(bytes)
        }))
    }
}

fn covers_file(path: &Path, parts: &[&SegmentRow]) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_file() {
        return false;
    }
    let mut spans: Vec<_> = parts.iter().map(|part| (part.begin, part.end)).collect();
    spans.sort_unstable();
    let mut end = 0i64;
    for (begin, next) in spans {
        if begin < 0 || next < begin || begin > end {
            return false;
        }
        end = end.max(next);
    }
    u64::try_from(end).ok() == Some(meta.len())
}
