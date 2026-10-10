/// What a grouped file is, from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileKind {
    Data,
    Par2,
    Nfo,
    Image,
    Video,
    Archive,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Data => "data",
            FileKind::Par2 => "par2",
            FileKind::Nfo => "nfo",
            FileKind::Image => "image",
            FileKind::Video => "video",
            FileKind::Archive => "archive",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "par2" => FileKind::Par2,
            "nfo" => FileKind::Nfo,
            "image" => FileKind::Image,
            "video" => FileKind::Video,
            "archive" => FileKind::Archive,
            _ => FileKind::Data,
        }
    }
}

/// One overview subject broken into the file, the part, and the article title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSegment {
    pub filename: Option<String>,
    pub file_index: Option<u32>,
    pub file_count: Option<u32>,
    pub part: Option<u32>,
    pub total: Option<u32>,
    pub kind: FileKind,
    pub title: String,
    /// Lowercased title, a newline, then the lowercased poster.
    pub group_key: String,
}

pub fn parse_subject(subject: &str, poster: &str) -> ParsedSegment {
    parse_subject_impl(subject, poster, true)
}

/// Match immutable overview records to catalog keys written before quoted names
/// containing spaces were supported. Use only as a fallback when reading a cache;
/// new catalog entries should always use `parse_subject`.
pub fn parse_subject_legacy(subject: &str, poster: &str) -> ParsedSegment {
    parse_subject_impl(subject, poster, false)
}

fn parse_subject_impl(subject: &str, poster: &str, spaced_names: bool) -> ParsedSegment {
    let filename = quoted_filename(subject, spaced_names).or_else(|| last_filename_token(subject));
    let (file_index, file_count) = bracket_counter(subject);
    let (part, total) = paren_counter(subject);
    let kind = filename.as_deref().map(classify).unwrap_or(FileKind::Data);
    let cleaned = clean_title(subject, filename.as_deref(), spaced_names);
    let title = if cleaned.is_empty() {
        filename
            .clone()
            .unwrap_or_else(|| subject.trim().to_string())
    } else {
        cleaned
    };
    let mut group_key = format!("{}\n{}", normalize(&title), normalize(poster));
    if spaced_names && filename.as_deref().is_some_and(|name| name.contains(' ')) {
        // Keep existing catalog identities stable as later parts arrive. The
        // corrected display title and full filename do not require reindexing.
        group_key = parse_subject_impl(subject, poster, false).group_key;
    }
    ParsedSegment {
        filename,
        file_index,
        file_count,
        part,
        total,
        kind,
        title,
        group_key,
    }
}

pub fn classify(filename: &str) -> FileKind {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    if ext == "par2" {
        FileKind::Par2
    } else if ext == "nfo" || ext == "sfv" {
        FileKind::Nfo
    } else if matches!(ext, "jpg" | "jpeg" | "png" | "webp" | "gif") {
        FileKind::Image
    } else if matches!(
        ext,
        "mp4"
            | "mkv"
            | "webm"
            | "mov"
            | "avi"
            | "m4v"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "3gp"
            | "m2ts"
            | "mts"
    ) {
        FileKind::Video
    } else if matches!(ext, "rar" | "zip" | "7z") || r00_ext(ext) {
        FileKind::Archive
    } else {
        FileKind::Data
    }
}

fn r00_ext(ext: &str) -> bool {
    let Some(rest) = ext.strip_prefix('r') else {
        return false;
    };
    rest.len() >= 2 && rest.chars().all(|c| c.is_ascii_digit())
}

fn quoted_filename(subject: &str, spaced_names: bool) -> Option<String> {
    let bytes = subject.as_bytes();
    let mut i = 0;
    let mut found = None;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if let Some(end) = bytes[i + 1..].iter().position(|b| *b == b'"') {
                let inner = &subject[i + 1..i + 1 + end];
                if looks_like_filename(inner, spaced_names) {
                    found = Some(inner.to_string());
                }
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn last_filename_token(subject: &str) -> Option<String> {
    subject
        .split_whitespace()
        .filter(|tok| {
            looks_like_filename(
                tok.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | ',')),
                false,
            )
        })
        .map(|tok| {
            tok.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | ','))
                .to_string()
        })
        .next_back()
}

fn looks_like_filename(token: &str, spaced_names: bool) -> bool {
    if token.contains('/')
        || token.contains('\\')
        || (!spaced_names && token.contains(' '))
        || token.chars().any(char::is_control)
    {
        return false;
    }
    let Some((stem, ext)) = token.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=8).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
}

fn bracket_counter(subject: &str) -> (Option<u32>, Option<u32>) {
    counter_between(subject, '[', ']')
}

fn paren_counter(subject: &str) -> (Option<u32>, Option<u32>) {
    counter_between(subject, '(', ')')
}

/// Last `n/m` inside the given brackets wins. File posts put the segment counter last.
fn counter_between(subject: &str, open: char, close: char) -> (Option<u32>, Option<u32>) {
    let mut found = (None, None);
    let chars: Vec<char> = subject.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == open {
            if let Some(rel) = chars[i + 1..].iter().position(|c| *c == close) {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                if let Some(pair) = parse_pair(inner.trim()) {
                    found = (Some(pair.0), Some(pair.1));
                }
                i += rel + 1;
                continue;
            }
        }
        i += 1;
    }
    found
}

fn parse_pair(text: &str) -> Option<(u32, u32)> {
    let (left, right) = text.split_once('/')?;
    let n: u32 = left.trim().parse().ok()?;
    let m: u32 = right.trim().parse().ok()?;
    if n == 0 || m == 0 {
        return None;
    }
    Some((n, m))
}

fn clean_title(subject: &str, filename: Option<&str>, spaced_names: bool) -> String {
    let mut out = String::new();
    let chars: Vec<char> = subject.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            if let Some(rel) = chars[i + 1..].iter().position(|c| *c == '"') {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                if filename == Some(inner.as_str()) || looks_like_filename(&inner, spaced_names) {
                    i += rel + 2;
                    out.push(' ');
                    continue;
                }
            }
        }
        if chars[i] == '[' || chars[i] == '(' {
            let close = if chars[i] == '[' { ']' } else { ')' };
            if let Some(rel) = chars[i + 1..].iter().position(|c| *c == close) {
                let inner: String = chars[i + 1..i + 1 + rel].iter().collect();
                if parse_pair(inner.trim()).is_some() {
                    i += rel + 2;
                    out.push(' ');
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    if let Some(name) = filename {
        out = remove_token(&out, name);
    }
    out = remove_token(&out, "yenc");
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| c == '-' || c == '|' || c == ':' || c.is_whitespace())
        .trim()
        .to_string()
}

fn remove_token(text: &str, token: &str) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        let trimmed = word
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | ',' | '-'));
        if trimmed.eq_ignore_ascii_case(token) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_spaced_names_remain_complete_without_changing_cached_identities() {
        let first = parse_subject(
            r#"Holiday [01/02] - "My holiday photo.JPG" yEnc (1/3)"#,
            "Ann",
        );
        let next = parse_subject(r#"Holiday [02/02] - "Min ferie æøå.JPG" yEnc (2/3)"#, "Ann");
        assert_eq!(first.filename.as_deref(), Some("My holiday photo.JPG"));
        assert_eq!(next.filename.as_deref(), Some("Min ferie æøå.JPG"));
        assert_eq!(first.title, "Holiday");
        assert_eq!(next.title, "Holiday");
        assert_eq!(first.kind, FileKind::Image);
        let legacy = parse_subject_legacy(
            r#"Holiday [01/02] - "My holiday photo.JPG" yEnc (1/3)"#,
            "Ann",
        );
        assert_eq!(legacy.filename.as_deref(), Some("photo.JPG"));
        assert_eq!(legacy.group_key, first.group_key);
    }

    #[test]
    fn quoted_rar_with_both_counters() {
        let parsed = parse_subject(r#"Name [01/42] - "file.part01.rar" yEnc (1/50)"#, "Foo");
        assert_eq!(parsed.filename.as_deref(), Some("file.part01.rar"));
        assert_eq!(parsed.file_index, Some(1));
        assert_eq!(parsed.file_count, Some(42));
        assert_eq!(parsed.part, Some(1));
        assert_eq!(parsed.total, Some(50));
        assert_eq!(parsed.kind, FileKind::Archive);
        assert_eq!(parsed.title, "Name");
        assert!(
            parsed.group_key.starts_with("name\nfoo"),
            "{}",
            parsed.group_key
        );
    }

    #[test]
    fn r00_title_falls_back_to_filename() {
        let parsed = parse_subject(r#""file.r00" yEnc (01/99)"#, "Foo");
        assert_eq!(parsed.filename.as_deref(), Some("file.r00"));
        assert_eq!(parsed.part, Some(1));
        assert_eq!(parsed.total, Some(99));
        assert_eq!(parsed.file_index, None);
        assert_eq!(parsed.kind, FileKind::Archive);
        assert_eq!(parsed.title, "file.r00");
    }

    #[test]
    fn paren_only_subject_has_no_filename() {
        let parsed = parse_subject("Holiday photo (1/3)", "Ann");
        assert_eq!(parsed.filename, None);
        assert_eq!(parsed.title, "Holiday photo");
        assert_eq!(parsed.part, Some(1));
        assert_eq!(parsed.total, Some(3));
        assert_eq!(parsed.kind, FileKind::Data);
        assert_eq!(parsed.group_key, "holiday photo\nann");
    }

    #[test]
    fn single_image_and_sidecar_kinds() {
        let image = parse_subject("cat.jpg", "Ann");
        assert_eq!(image.filename.as_deref(), Some("cat.jpg"));
        assert_eq!(image.kind, FileKind::Image);
        assert_eq!(image.part, None);
        assert_eq!(image.title, "cat.jpg");

        assert_eq!(parse_subject("set.par2", "A").kind, FileKind::Par2);
        assert_eq!(parse_subject("notes.nfo", "A").kind, FileKind::Nfo);
        assert_eq!(parse_subject("a.sfv", "A").kind, FileKind::Nfo);
    }

    #[test]
    fn multipart_video_subjects_and_case_insensitive_extensions() {
        for filename in [
            "clip.MP4",
            "clip.mkv",
            "clip.webm",
            "clip.mov",
            "clip.avi",
            "clip.m4v",
        ] {
            let subject = format!("Clip - \"{filename}\" yEnc (2/8)");
            let parsed = parse_subject(&subject, "Poster");
            assert_eq!(parsed.kind, FileKind::Video);
            assert_eq!(parsed.total, Some(8));
            assert_eq!(FileKind::parse(parsed.kind.as_str()), FileKind::Video);
        }
        assert_eq!(classify("notes.mp4.txt"), FileKind::Data);
    }

    #[test]
    fn key_follows_poster_and_ignores_part_numbers() {
        let a = parse_subject(r#"Name [01/42] - "file.part01.rar" yEnc (1/50)"#, "Foo");
        let b = parse_subject(r#"Name [02/42] - "file.part02.rar" yEnc (2/50)"#, "Foo");
        let c = parse_subject(r#"Name [01/42] - "file.part01.rar" yEnc (1/50)"#, "Bar");
        assert_eq!(a.group_key, b.group_key);
        assert_ne!(a.group_key, c.group_key);
        assert_eq!(a.title, "Name");
        assert_eq!(b.title, "Name");
    }
}
