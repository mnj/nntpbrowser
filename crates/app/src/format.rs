//! Display helpers that do not touch the window.

pub fn format_bytes(n: i64) -> String {
    let mut value = n.max(0) as f64;
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn age(unix: i64, now: i64) -> String {
    if unix <= 0 {
        return "unknown date".into();
    }
    let delta = now.saturating_sub(unix);
    if delta < 60 {
        "just now".into()
    } else if delta < 3600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

pub fn grid_columns(width: f32, min_tile: f32) -> usize {
    if min_tile <= 1.0 {
        return 1;
    }
    (width / min_tile).floor().max(1.0) as usize
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedFilter {
    All,
    Images,
    Media,
    Sets,
    Repair,
    Partial,
}

pub fn passes_filter(
    has_image: bool,
    has_video: bool,
    has_par2: bool,
    incomplete: bool,
    file_count: i64,
    filter: FeedFilter,
) -> bool {
    match filter {
        FeedFilter::All => true,
        FeedFilter::Images => has_image,
        FeedFilter::Media => has_image || has_video,
        FeedFilter::Sets => file_count > 1,
        FeedFilter::Repair => has_par2,
        FeedFilter::Partial => incomplete,
    }
}

/// UTC timestamp as `YYYY-MM-DD HH:MM`. Zero and negative values have no date.
pub fn format_stamp(unix: i64) -> String {
    if unix <= 0 {
        return "—".into();
    }
    let (year, month, day, hour, minute) = civil_utc(unix);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn civil_utc(unix: i64) -> (i32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86_400);
    let tod = unix.rem_euclid(86_400) as u32;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp as i64 + if mp < 10 { 3 } else { -9 }) as u32;
    if month <= 2 {
        year += 1;
    }
    (year as i32, month, day, hour, minute)
}

pub fn scrub(text: &str, secrets: &[impl AsRef<str>]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        let secret = secret.as_ref();
        if secret.len() >= 3 {
            out = out.replace(secret, "••••");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_age_and_columns() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(age(0, 10), "unknown date");
        assert_eq!(age(100, 160), "1m ago");
        assert_eq!(grid_columns(400.0, 180.0), 2);
        assert_eq!(grid_columns(100.0, 180.0), 1);
    }

    #[test]
    fn secret_scrub() {
        assert_eq!(
            scrub("connection failed: s3cret-pass", &["s3cret-pass"]),
            "connection failed: ••••"
        );
    }

    #[test]
    fn stamps_and_filters() {
        assert_eq!(format_stamp(0), "—");
        assert_eq!(format_stamp(1_700_000_000), "2023-11-14 22:13");
        assert!(passes_filter(
            true,
            false,
            false,
            false,
            1,
            FeedFilter::Images
        ));
        assert!(!passes_filter(
            false,
            false,
            true,
            false,
            3,
            FeedFilter::Images
        ));
        assert!(passes_filter(
            false,
            false,
            true,
            false,
            3,
            FeedFilter::Repair
        ));
        assert!(passes_filter(
            false,
            false,
            false,
            true,
            1,
            FeedFilter::Partial
        ));
        assert!(passes_filter(
            true,
            false,
            false,
            false,
            4,
            FeedFilter::Sets
        ));
        assert!(!passes_filter(
            true,
            false,
            false,
            false,
            1,
            FeedFilter::Sets
        ));
    }
}
