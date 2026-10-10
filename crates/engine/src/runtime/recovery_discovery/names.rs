use std::collections::BTreeSet;

pub(super) fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        && !name.ends_with(['.', ' '])
        && !name.chars().any(char::is_control)
}

/// Return the exact set stem and advertised block count; no prefix matching.
pub(super) fn parity_name(name: &str) -> Option<(String, Option<u32>)> {
    if !safe_name(name) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let base = lower.strip_suffix(".par2")?;
    if base.is_empty() {
        return None;
    }
    if let Some((stem, _)) = base.rsplit_once(".vol").filter(|(_, volume)| {
        volume.as_bytes().first().is_some_and(u8::is_ascii_digit) || volume.contains('+')
    }) {
        if stem.is_empty() {
            return None;
        }
        let (first, end) = store::recovery_volume_range(name)?;
        Some((stem.into(), Some(u32::try_from(end - first).ok()?)))
    } else {
        Some((base.into(), None))
    }
}

pub(super) fn inferred_stems(payload_names: &[String]) -> BTreeSet<String> {
    let mut stems = BTreeSet::new();
    for name in payload_names.iter().filter(|name| safe_name(name)) {
        let lower = name.to_ascii_lowercase();
        if parity_name(&lower).is_some() {
            continue;
        }
        // Writers use both "archive.rar.par2" and "archive.par2".
        stems.insert(lower.clone());
        if let Some((stem, extension)) = lower.rsplit_once('.') {
            stems.insert(stem.into());
            if extension == "rar" {
                if let Some((root, part)) = stem.rsplit_once(".part") {
                    if !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()) {
                        stems.insert(root.into());
                    }
                }
            }
        }
    }
    stems.remove("");
    stems
}

pub(super) fn normalized_poster(poster: &str) -> String {
    poster
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(super) fn malformed_part_counter(subject: &str) -> bool {
    subject
        .split('(')
        .skip(1)
        .filter_map(|tail| tail.split_once(')').map(|pair| pair.0))
        .filter_map(|inner| inner.split_once('/'))
        .any(|(left, right)| {
            let left = left.trim();
            let right = right.trim();
            let numeric =
                |side: &str| !side.is_empty() && side.bytes().all(|byte| byte.is_ascii_digit());
            if !numeric(left) && !numeric(right) {
                return false;
            }
            match (left.parse::<u32>(), right.parse::<u32>()) {
                (Ok(part), Ok(total)) => part == 0 || total == 0 || part > total,
                _ => true,
            }
        })
}
