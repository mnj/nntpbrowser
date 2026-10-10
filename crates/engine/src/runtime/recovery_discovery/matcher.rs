use super::names::{
    inferred_stems, malformed_part_counter, normalized_poster, parity_name, safe_name,
};
use std::collections::{BTreeMap, BTreeSet};
use store::{NewSegment, Record, SegmentRow};

const MATCHED_HEADERS: usize = 65_536;
const CANDIDATE_FILES: usize = 512;

#[derive(Clone, Debug)]
pub(in crate::runtime) struct RecoveryCandidate {
    pub expected_parts: u32,
    pub recovery_blocks: Option<u32>,
    pub segments: Vec<NewSegment>,
}

struct Candidate {
    expected: u32,
    blocks: Option<u32>,
    parts: BTreeMap<u32, NewSegment>,
    conflicting_total: bool,
}

pub(in crate::runtime) struct Discovery {
    group_key: String,
    poster: String,
    stems: BTreeSet<String>,
    payload_names: BTreeSet<String>,
    known_parts: BTreeSet<(String, i64)>,
    known_message_ids: BTreeSet<String>,
    candidates: BTreeMap<String, Candidate>,
    payload: BTreeMap<(String, u32), NewSegment>,
    retained: usize,
    posted_at: Option<i64>,
    anchors: Vec<u64>,
}

impl Discovery {
    pub fn new(
        group_key: &str,
        poster: &str,
        payload_names: &[String],
        known_segments: &[SegmentRow],
    ) -> Result<Self, String> {
        if group_key.is_empty() || poster.trim().is_empty() {
            return Err("recovery discovery needs the original article identity and poster".into());
        }
        let mut known_stems: BTreeSet<_> = known_segments
            .iter()
            .filter_map(|segment| parity_name(&segment.filename).map(|name| name.0))
            .collect();
        if known_stems.is_empty() {
            known_stems = inferred_stems(payload_names);
            if let Some(title) = group_key
                .split('\n')
                .next()
                .filter(|title| safe_name(title))
            {
                known_stems.insert(title.to_ascii_lowercase());
            }
        }
        let mut anchors = known_segments
            .iter()
            .filter_map(|part| part.article.and_then(|article| u64::try_from(article).ok()))
            .collect::<Vec<_>>();
        anchors.sort_unstable();
        anchors.dedup();
        Ok(Self {
            group_key: group_key.into(),
            poster: normalized_poster(poster),
            stems: known_stems,
            payload_names: payload_names
                .iter()
                .filter(|name| safe_name(name))
                .cloned()
                .collect(),
            known_parts: known_segments
                .iter()
                .filter(|segment| {
                    matches!(segment.state.as_str(), "written" | "pending" | "fetching")
                })
                .map(|segment| (segment.filename.clone(), segment.part))
                .collect(),
            known_message_ids: known_segments
                .iter()
                .map(|segment| segment.message_id.clone())
                .collect(),
            candidates: BTreeMap::new(),
            payload: BTreeMap::new(),
            retained: 0,
            posted_at: None,
            anchors,
        })
    }

    pub fn with_posted_at(mut self, posted_at: Option<i64>) -> Self {
        self.posted_at = posted_at.filter(|&stamp| stamp > 0);
        self
    }

    /// Restore job-owned candidates already matched before a successful search
    /// range was marked durable. These IDs are not newly discovered headers.
    pub fn seed_candidates(&mut self, parts: &[(NewSegment, u32)]) -> Result<(), String> {
        for (segment, expected) in parts {
            let Some((stem, blocks)) = parity_name(&segment.filename) else {
                return Err("invalid persisted recovery filename".into());
            };
            if !self.stems.contains(&stem)
                || *expected == 0
                || *expected as usize > MATCHED_HEADERS
                || segment.part <= 0
                || segment.part > i64::from(*expected)
                || !valid_message_id(&segment.message_id)
                || self.retained >= MATCHED_HEADERS
                || (!self.candidates.contains_key(&segment.filename)
                    && self.candidates.len() >= CANDIDATE_FILES)
            {
                return Err("invalid or excessive persisted recovery parts".into());
            }
            let candidate = self
                .candidates
                .entry(segment.filename.clone())
                .or_insert_with(|| Candidate {
                    expected: *expected,
                    blocks,
                    parts: BTreeMap::new(),
                    conflicting_total: false,
                });
            if candidate.expected != *expected {
                return Err("persisted recovery parts disagree on their total".into());
            }
            if let std::collections::btree_map::Entry::Vacant(entry) =
                candidate.parts.entry(segment.part as u32)
            {
                entry.insert(segment.clone());
                self.retained += 1;
            }
        }
        Ok(())
    }

    pub fn ingest(&mut self, record: Record) {
        if self.retained >= MATCHED_HEADERS
            || record.subject.len() > 4096
            || record.from.len() > 1024
            || !valid_message_id(&record.message_id)
            || malformed_part_counter(&record.subject)
            || normalized_poster(&record.from) != self.poster
            || self.posted_at.is_some_and(|stamp| {
                record.date_unix <= 0 || record.date_unix.abs_diff(stamp) > 7 * 86_400
            })
        {
            return;
        }
        let parsed = store::parse_subject(&record.subject, &record.from);
        if parsed.group_key != self.group_key {
            return;
        }
        let Some(filename) = parsed.filename.filter(|name| safe_name(name)) else {
            return;
        };
        let (part, total) = match (parsed.part, parsed.total) {
            (Some(part), Some(total)) => (part, total),
            (None, None) => (1, 1),
            _ => return,
        };
        if part == 0 || total == 0 || part > total || total as usize > MATCHED_HEADERS {
            return;
        }
        let segment = NewSegment {
            filename: filename.clone(),
            part: i64::from(part),
            message_id: record.message_id,
            begin: 0,
            end: 0,
            article: i64::try_from(record.article).ok(),
        };
        let distance = |part: &NewSegment| {
            let Some(article) = part.article.and_then(|article| u64::try_from(article).ok()) else {
                return u64::MAX;
            };
            let index = self.anchors.partition_point(|&anchor| anchor < article);
            self.anchors
                .get(index)
                .into_iter()
                .chain(
                    index
                        .checked_sub(1)
                        .and_then(|index| self.anchors.get(index)),
                )
                .map(|anchor| anchor.abs_diff(article))
                .min()
                .unwrap_or(u64::MAX)
        };
        if let Some((stem, blocks)) = parity_name(&filename) {
            if !self.stems.contains(&stem)
                || (!self.candidates.contains_key(&filename)
                    && self.candidates.len() >= CANDIDATE_FILES)
            {
                return;
            }
            let candidate = self
                .candidates
                .entry(filename)
                .or_insert_with(|| Candidate {
                    expected: total,
                    blocks,
                    parts: BTreeMap::new(),
                    conflicting_total: false,
                });
            if candidate.expected != total {
                candidate.conflicting_total = true;
                return;
            }
            match candidate.parts.entry(part) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(segment);
                    self.retained += 1;
                }
                std::collections::btree_map::Entry::Occupied(mut entry)
                    if distance(&segment) < distance(entry.get()) =>
                {
                    entry.insert(segment);
                }
                _ => {}
            }
        } else if self.payload_names.contains(&filename)
            && !self.known_message_ids.contains(&segment.message_id)
            && !self
                .known_parts
                .contains(&(filename.clone(), i64::from(part)))
        {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.payload.entry((filename, part))
            {
                entry.insert(segment);
                self.retained += 1;
            }
        }
    }

    /// Persist incomplete metadata too, so later scans/restarts can finish it.
    pub fn partial_candidates(&self) -> Vec<RecoveryCandidate> {
        self.candidates
            .iter()
            .filter(|(_, candidate)| !candidate.conflicting_total)
            .map(|(_, candidate)| RecoveryCandidate {
                expected_parts: candidate.expected,
                recovery_blocks: candidate.blocks,
                segments: candidate.parts.values().cloned().collect(),
            })
            .collect()
    }

    pub fn missing_payload_segments(&self) -> Vec<NewSegment> {
        self.payload.values().cloned().collect()
    }
}

fn valid_message_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 497 && !id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
}
