//! NZB XML → the same file/segment shape the downloader uses for articles.

use quick_xml::events::Event;
use quick_xml::Reader;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NzbError {
    #[error("nzb xml: {0}")]
    Xml(String),
    #[error("nzb has no files")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NzbSegment {
    pub bytes: u64,
    pub number: u32,
    pub message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NzbFile {
    pub subject: String,
    pub poster: String,
    pub date: i64,
    pub groups: Vec<String>,
    pub segments: Vec<NzbSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nzb {
    pub files: Vec<NzbFile>,
    pub password: Option<String>,
}

pub fn parse(xml: &str) -> Result<Nzb, NzbError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut files = Vec::new();
    let mut password = None;
    let mut buf = Vec::new();

    let mut file: Option<FileBuild> = None;
    let mut in_groups = false;
    let mut in_segments = false;
    let mut text_target: Option<TextTarget> = None;
    let mut meta_type: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "file" => {
                        file = Some(FileBuild {
                            subject: attr(&e, b"subject").unwrap_or_default(),
                            poster: attr(&e, b"poster").unwrap_or_default(),
                            date: attr(&e, b"date").and_then(|s| s.parse().ok()).unwrap_or(0),
                            groups: Vec::new(),
                            segments: Vec::new(),
                        });
                    }
                    "groups" => in_groups = true,
                    "segments" => in_segments = true,
                    "group" if in_groups => text_target = Some(TextTarget::Group),
                    "segment" if in_segments => {
                        let bytes = attr(&e, b"bytes").and_then(|s| s.parse().ok()).unwrap_or(0);
                        let number = attr(&e, b"number")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        if let Some(file) = file.as_mut() {
                            file.segments.push(NzbSegment {
                                bytes,
                                number,
                                message_id: String::new(),
                            });
                        }
                        text_target = Some(TextTarget::Segment);
                    }
                    "meta" => {
                        meta_type = attr(&e, b"type");
                        text_target = Some(TextTarget::Meta);
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if local_name(e.name().as_ref()) == "meta"
                    && attr(&e, b"type").as_deref() == Some("password")
                {
                    password = attr(&e, b"password").or_else(|| attr(&e, b"value"));
                }
            }
            Ok(Event::Text(t)) => {
                let text = t
                    .unescape()
                    .map_err(|err| NzbError::Xml(err.to_string()))?
                    .into_owned();
                match text_target {
                    Some(TextTarget::Group) => {
                        if let Some(file) = file.as_mut() {
                            if !text.is_empty() {
                                file.groups.push(text);
                            }
                        }
                    }
                    Some(TextTarget::Segment) => {
                        if let Some(file) = file.as_mut() {
                            if let Some(seg) = file.segments.last_mut() {
                                seg.message_id = strip_msgid(&text);
                            }
                        }
                    }
                    Some(TextTarget::Meta)
                        if meta_type.as_deref() == Some("password") && !text.is_empty() =>
                    {
                        password = Some(text);
                    }
                    None | Some(TextTarget::Meta) => {}
                }
            }
            Ok(Event::End(e)) => match local_name(e.name().as_ref()).as_str() {
                "file" => {
                    if let Some(built) = file.take() {
                        files.push(built.finish());
                    }
                }
                "groups" => in_groups = false,
                "segments" => in_segments = false,
                "group" | "segment" | "meta" => {
                    text_target = None;
                    if local_name(e.name().as_ref()) == "meta" {
                        meta_type = None;
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(err) => return Err(NzbError::Xml(err.to_string())),
            _ => {}
        }
        buf.clear();
    }

    if files.is_empty() {
        return Err(NzbError::Empty);
    }
    Ok(Nzb { files, password })
}

struct FileBuild {
    subject: String,
    poster: String,
    date: i64,
    groups: Vec<String>,
    segments: Vec<NzbSegment>,
}

impl FileBuild {
    fn finish(mut self) -> NzbFile {
        self.segments.sort_by_key(|s| s.number);
        NzbFile {
            subject: self.subject,
            poster: self.poster,
            date: self.date,
            groups: self.groups,
            segments: self.segments,
        }
    }
}

enum TextTarget {
    Group,
    Segment,
    Meta,
}

fn local_name(qname: &[u8]) -> String {
    let raw = String::from_utf8_lossy(qname);
    match raw.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => raw.into_owned(),
    }
}

fn attr(e: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.local_name().as_ref() == key {
            return attr
                .unescape_value()
                .ok()
                .map(|v| v.into_owned())
                .filter(|v| !v.is_empty());
        }
    }
    None
}

fn strip_msgid(raw: &str) -> String {
    let t = raw.trim();
    t.trim_start_matches('<').trim_end_matches('>').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<nzb xmlns="http://www.newzbin.com/DTD/2003/nzb">
  <head>
    <meta type="password">secret</meta>
  </head>
  <file poster="a@b" date="1700000000" subject="Name [01/1] - &quot;file.rar&quot; yEnc (1/2)">
    <groups><group>alt.binaries.test</group></groups>
    <segments>
      <segment bytes="10" number="2">&lt;part2@ex&gt;</segment>
      <segment bytes="20" number="1">&lt;part1@ex&gt;</segment>
    </segments>
  </file>
</nzb>"#;

    #[test]
    fn parses_password_order_and_msgid() {
        let nzb = parse(SAMPLE).unwrap();
        assert_eq!(nzb.password.as_deref(), Some("secret"));
        assert_eq!(nzb.files.len(), 1);
        let file = &nzb.files[0];
        assert_eq!(file.subject, "Name [01/1] - \"file.rar\" yEnc (1/2)");
        assert_eq!(file.groups, vec!["alt.binaries.test".to_string()]);
        assert_eq!(file.segments[0].number, 1);
        assert_eq!(file.segments[0].message_id, "part1@ex");
        assert_eq!(file.segments[1].message_id, "part2@ex");
        assert_eq!(file.segments[0].bytes, 20);
    }

    #[test]
    fn empty_is_an_error() {
        assert!(parse("<nzb></nzb>").is_err());
    }
}
