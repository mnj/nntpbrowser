use super::{names::volume_range, *};
use store::{JobRecoveryContext, NewSegment};

pub(in crate::runtime) type CandidateSeed = (NewSegment, Option<i64>, i64);

pub(in crate::runtime) fn split_parity(
    parts: &[NewSegment],
    expected: &HashMap<String, i64>,
) -> (Vec<NewSegment>, Vec<CandidateSeed>) {
    if !parts
        .iter()
        .any(|part| store::classify(&part.filename) != store::FileKind::Par2)
        && !expected
            .keys()
            .any(|name| store::classify(name) != store::FileKind::Par2)
    {
        return (parts.to_vec(), Vec::new());
    }
    let mut active = Vec::new();
    let mut deferred = Vec::new();
    for part in parts {
        if let Some((first, end)) = volume_range(&part.filename) {
            let total = expected
                .get(&part.filename)
                .copied()
                .filter(|total| *total > 0)
                .unwrap_or_else(|| {
                    parts
                        .iter()
                        .filter(|p| p.filename == part.filename)
                        .map(|p| p.part)
                        .max()
                        .unwrap_or(1)
                });
            deferred.push((part.clone(), i64::try_from(end - first).ok(), total));
        } else {
            active.push(part.clone());
        }
    }
    (active, deferred)
}

impl Worker {
    pub(in crate::runtime) fn recovery_source(
        &self,
        post_id: Option<i64>,
    ) -> Result<JobRecoveryContext, String> {
        let source = post_id.and_then(|id| self.find_post(id));
        let (server_id, group_id, group, poster, group_key, posted_at) =
            if let Some((server, group, post)) = source {
                (
                    Some(server.id),
                    Some(group.id),
                    group.name,
                    post.poster,
                    post.group_key,
                    (post.posted_at > 0).then_some(post.posted_at),
                )
            } else {
                (
                    None,
                    None,
                    String::new(),
                    String::new(),
                    String::new(),
                    None,
                )
            };
        Ok(JobRecoveryContext {
            server_id,
            group_id,
            group,
            poster,
            group_key,
            posted_at,
            set_id: None,
            discovery_exhausted: false,
            discovery_ranges: Vec::new(),
        })
    }

    pub(in crate::runtime) fn initialize_recovery(
        &self,
        job: &store::JobRow,
    ) -> Result<(), String> {
        if self
            .catalog
            .recovery_context(job.id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Ok(());
        }
        let context = self.recovery_source(job.post_id)?;
        let parts = self
            .catalog
            .segments_for_job(job.id)
            .map_err(|e| e.to_string())?;
        let expected: HashMap<_, _> = job
            .post_id
            .map(|id| self.catalog.files_for_post(id))
            .transpose()
            .map_err(|e| e.to_string())?
            .unwrap_or_default()
            .into_iter()
            .map(|file| (file.name, file.expected_segments))
            .collect();
        let has_payload = parts
            .iter()
            .any(|part| store::classify(&part.filename) != store::FileKind::Par2);
        if has_payload {
            let deferred: Vec<_> = parts
                .iter()
                .filter_map(|part| {
                    let (first, end) = volume_range(&part.filename)?;
                    let total = expected
                        .get(&part.filename)
                        .copied()
                        .filter(|total| *total > 0)
                        .unwrap_or_else(|| {
                            parts
                                .iter()
                                .filter(|p| p.filename == part.filename)
                                .map(|p| p.part)
                                .max()
                                .unwrap_or(1)
                        });
                    Some((part.id, i64::try_from(end - first).ok(), total))
                })
                .collect();
            self.catalog
                .defer_job_segments(job.id, &deferred)
                .map_err(|e| e.to_string())?;
        }
        self.catalog
            .set_recovery_context(job.id, &context)
            .map_err(|e| e.to_string())
    }
}
