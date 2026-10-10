use super::NewSegment;

/// Durable source identity and completed header-discovery spans for one job.
/// Deleted source rows become None and exhaust discovery; saved files survive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRecoveryContext {
    pub server_id: Option<i64>,
    pub group_id: Option<i64>,
    pub group: String,
    pub poster: String,
    pub group_key: String,
    pub posted_at: Option<i64>,
    pub set_id: Option<String>,
    pub discovery_exhausted: bool,
    pub discovery_ranges: Vec<(u64, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryCandidate {
    pub id: i64,
    pub job_id: i64,
    pub segment: NewSegment,
    /// Filename hint only; the engine validates actual PAR2 contents.
    pub recovery_blocks: Option<i64>,
    pub expected_parts: i64,
    /// deferred, requested or exhausted. A restart never repeats a request.
    pub state: String,
}

/// Parse a conventional PAR2 volume's hinted exponent range, end-exclusive.
/// A filename is only a scheduling hint; PAR2 packet contents prove identity.
pub fn recovery_volume_range(name: &str) -> Option<(u64, u64)> {
    if name.len() > 255 || !super::queue::valid_filename(name) {
        return None;
    }
    let name = name.to_ascii_lowercase();
    let stem = name.strip_suffix(".par2")?;
    let (_, volume) = stem.rsplit_once(".vol")?;
    let (start, count) = volume.split_once('+')?;
    if start.is_empty()
        || count.is_empty()
        || !start
            .bytes()
            .chain(count.bytes())
            .all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let start = start.parse::<u32>().ok()?;
    let count = count.parse::<u32>().ok()?;
    (count > 0).then_some((u64::from(start), u64::from(start.checked_add(count)?)))
}

pub fn recovery_volume_blocks(name: &str) -> Option<i64> {
    let (start, end) = recovery_volume_range(name)?;
    i64::try_from(end - start).ok()
}
