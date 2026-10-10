//! Resolve only the current requested files with one streamed scan per group.
use super::{Identity, PreviewIntent, PreviewRequest, MAX_PREVIEW};
use crate::runtime::Worker;
use std::collections::{HashMap, HashSet};
use store::{FileKind, FileRow, GroupRow, NewSegment, PostRow, ServerRow};

struct Seed {
    server: ServerRow,
    group: GroupRow,
    post: PostRow,
    file: FileRow,
    generation: u64,
    cap: u64,
    segments: Vec<NewSegment>,
    header_name: Option<String>,
    error: Option<String>,
}

impl Worker {
    pub(super) fn prepare_preview(
        &self,
        post_id: i64,
        max_bytes: u64,
        generation: u64,
    ) -> Result<PreviewRequest, String> {
        self.prepare_preview_batch(vec![PreviewIntent {
            post_id,
            max_bytes,
            generation,
        }])
        .pop()
        .unwrap()
        .1
    }

    pub(super) fn prepare_preview_batch(
        &self,
        intents: Vec<PreviewIntent>,
    ) -> Vec<(i64, Result<PreviewRequest, String>)> {
        let mut seeds = intents
            .into_iter()
            .map(|intent| (intent.post_id, self.preview_seed(intent)))
            .collect::<Vec<_>>();
        let mut groups: HashMap<(i64, String), Vec<usize>> = HashMap::new();
        for (index, (_, seed)) in seeds.iter().enumerate() {
            if let Ok(seed) = seed {
                groups
                    .entry((seed.server.id, seed.group.name.clone()))
                    .or_default()
                    .push(index);
            }
        }
        for ((server_id, group), indices) in groups {
            let mut wanted: HashMap<(String, String), usize> = HashMap::new();
            for &index in &indices {
                let seed = seeds[index].1.as_ref().unwrap();
                wanted.insert((seed.post.group_key.clone(), seed.file.name.clone()), index);
            }
            let scanned = self.headers.for_each(server_id, &group, |record| {
                let modern = store::parse_subject(&record.subject, &record.from);
                let legacy = store::parse_subject_legacy(&record.subject, &record.from);
                let mut matched = HashSet::new();
                for parsed in [&modern, &legacy] {
                    let Some(name) = &parsed.filename else {
                        continue;
                    };
                    let Some(&index) = wanted.get(&(parsed.group_key.clone(), name.clone())) else {
                        continue;
                    };
                    if !matched.insert(index) {
                        continue;
                    }
                    let seed = seeds[index].1.as_mut().unwrap();
                    if seed.segments.len() >= 4096 {
                        seed.error = Some("media has too many parts for a bounded preview".into());
                        continue;
                    }
                    seed.header_name.get_or_insert_with(|| {
                        modern
                            .filename
                            .clone()
                            .unwrap_or_else(|| seed.file.name.clone())
                    });
                    seed.segments.push(NewSegment {
                        filename: seed.file.name.clone(),
                        part: modern.part.unwrap_or(1) as i64,
                        message_id: record.message_id.clone(),
                        begin: 0,
                        end: 0,
                        article: i64::try_from(record.article).ok(),
                    });
                }
            });
            let error = match scanned {
                Ok(damaged) if damaged.is_empty() => None,
                Ok(_) => Some("stored media headers contain damaged blocks".into()),
                Err(error) => Some(error.to_string()),
            };
            if let Some(error) = error {
                for index in indices {
                    if let Ok(seed) = &mut seeds[index].1 {
                        seed.error = Some(error.clone());
                    }
                }
            }
        }
        seeds
            .into_iter()
            .map(|(id, seed)| (id, seed.and_then(Seed::finish)))
            .collect()
    }

    fn preview_seed(&self, intent: PreviewIntent) -> Result<Seed, String> {
        let cap = intent.max_bytes.min(MAX_PREVIEW);
        let (server, group, post) = self
            .find_post(intent.post_id)
            .ok_or("article was not found")?;
        let files = self
            .catalog
            .files_for_post(intent.post_id)
            .map_err(|error| error.to_string())?;
        let file = files
            .into_iter()
            .filter(|file| {
                matches!(file.kind, FileKind::Image | FileKind::Video)
                    && file.size > 0
                    && (file.size as u64) < cap
            })
            .min_by_key(|file| (file.kind != FileKind::Image, file.id))
            .ok_or("no image or video with a known size below the preview limit")?;
        Ok(Seed {
            server,
            group,
            post,
            file,
            generation: intent.generation,
            cap,
            segments: Vec::new(),
            header_name: None,
            error: None,
        })
    }
}

impl Seed {
    fn finish(mut self) -> Result<PreviewRequest, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        self.segments.sort_by_key(|segment| segment.part);
        self.segments.dedup_by(|a, b| a.part == b.part);
        if self.segments.len() as i64 != self.file.expected_segments.max(1)
            || !self
                .segments
                .iter()
                .enumerate()
                .all(|(index, segment)| segment.part == index as i64 + 1)
        {
            return Err("media headers are incomplete; fetch more headers first".into());
        }
        let identity = Identity {
            host: self.server.host,
            port: self.server.port,
            tls: self.server.tls,
            starttls: self.server.starttls,
            group: self.group.name,
            key: self.post.group_key,
            name: self.header_name.unwrap_or(self.file.name),
            parts: self
                .segments
                .iter()
                .map(|segment| (segment.part, segment.message_id.clone()))
                .collect(),
        };
        Ok(PreviewRequest {
            post_id: self.post.id,
            generation: self.generation,
            cap: self.cap,
            size: self.file.size as u64,
            identity,
            segments: self.segments,
        })
    }
}
