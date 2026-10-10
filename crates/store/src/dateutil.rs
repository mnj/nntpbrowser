use chrono::{DateTime, FixedOffset};

/// Overview dates are RFC 5322, or the shorter `dd Mon yyyy hh:mm:ss ±zzzz` form.
/// `GMT` and `UTC` are accepted as `+0000`. Failure means the caller stores unix 0
/// and keeps the raw text.
pub fn parse_overview_date(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(ts) = parse_one(raw) {
        return Some(ts);
    }
    let folded = fold_zone(raw)?;
    parse_one(&folded)
}

fn parse_one(raw: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc2822(raw) {
        return Some(dt.timestamp());
    }
    for fmt in ["%d %b %Y %H:%M:%S %z", "%a, %d %b %Y %H:%M:%S %z"] {
        if let Ok(dt) = DateTime::parse_from_str(raw, fmt) {
            let dt: DateTime<FixedOffset> = dt;
            return Some(dt.timestamp());
        }
    }
    None
}

fn fold_zone(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end();
    let rest = trimmed
        .strip_suffix("GMT")
        .or_else(|| trimmed.strip_suffix("UTC"))?;
    Some(format!("{} +0000", rest.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc2822_and_numeric_and_gmt() {
        let rfc = parse_overview_date("Tue, 10 Mar 2026 12:00:00 +0000").unwrap();
        let short = parse_overview_date("10 Mar 2026 12:00:00 +0000").unwrap();
        let gmt = parse_overview_date("10 Mar 2026 12:00:00 GMT").unwrap();
        let utc = parse_overview_date("Tue, 10 Mar 2026 12:00:00 UTC").unwrap();
        assert_eq!(rfc, short);
        assert_eq!(rfc, gmt);
        assert_eq!(rfc, utc);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_overview_date("not a date").is_none());
        assert!(parse_overview_date("").is_none());
    }
}
