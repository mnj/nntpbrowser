//! RFC 3977 reader operations. Session selection remains on this connection.

use super::*;
mod discovery;
mod fields;
mod selection;
mod types;
pub use types::*;
use types::{invalid, token};

impl Connection {
    /// None uses the current article; Message-ID retrieval does not select a group.
    pub async fn article(&mut self, id: Option<&ArticleRef>) -> Result<Article, Error> {
        self.article_limited(id, usize::MAX).await
    }

    /// An oversized reply leaves this connection unusable: discard it on error.
    pub async fn article_limited(
        &mut self,
        id: Option<&ArticleRef>,
        max_bytes: usize,
    ) -> Result<Article, Error> {
        let command = article_command("ARTICLE", id)?;
        self.writer.write_bytes(command.as_bytes()).await?;
        let (code, text) = self.reader.read_status().await?;
        if code != 220 {
            return Err(Error::from_status(code, text));
        }
        let identity = parse_identity(&text)?;
        let bytes = self.read_section_limited(max_bytes, "ARTICLE").await?;
        Ok(Article { identity, bytes })
    }

    pub async fn head_current(&mut self) -> Result<Vec<u8>, Error> {
        let (_, lines) = self.reader_data("HEAD\r\n", 221).await?;
        Ok(join_lines(&lines))
    }

    pub async fn body_current(&mut self, max_bytes: usize) -> Result<Vec<u8>, Error> {
        self.writer.write_bytes(b"BODY\r\n").await?;
        let (code, text) = self.reader.read_status().await?;
        if code != 222 {
            return Err(Error::from_status(code, text));
        }
        self.read_section_limited(max_bytes, "BODY").await
    }

    pub async fn stat_identity(
        &mut self,
        id: Option<&ArticleRef>,
    ) -> Result<ArticleIdentity, Error> {
        let (code, text, _) = self.command(&article_command("STAT", id)?).await?;
        if code != 223 {
            return Err(Error::from_status(code, text));
        }
        parse_identity(&text)
    }

    /// Request a fresh capability snapshot (the optional keyword is reserved by RFC3977).
    pub async fn request_capabilities(
        &mut self,
        keyword: Option<&str>,
    ) -> Result<Capabilities, Error> {
        let command = match keyword {
            Some(keyword) => format!("CAPABILITIES {}\r\n", token(keyword)?),
            None => "CAPABILITIES\r\n".into(),
        };
        let (_, lines) = self.reader_data(&command, 101).await?;
        self.caps = parse_capabilities(&lines);
        self.legacy_overview = self.caps.supports("XOVER") && !self.caps.supports("OVER");
        self.overview = None;
        Ok(self.caps.clone())
    }

    /// Explicit mode switch refreshes capabilities and invalidates overview format.
    pub async fn mode_reader(&mut self) -> Result<bool, Error> {
        if self.tls_active {
            return Err(invalid("MODE READER is forbidden after TLS is established"));
        }
        let (code, text, _) = self.command("MODE READER\r\n").await?;
        if !matches!(code, 200 | 201) {
            return Err(Error::from_status(code, text));
        }
        self.refresh_capabilities(false).await?;
        self.overview = None;
        Ok(code == 200)
    }

    async fn reader_data(
        &mut self,
        command: &str,
        expected: u16,
    ) -> Result<(String, Vec<Vec<u8>>), Error> {
        let phase = command_phase(command);
        trace_request(phase);
        self.writer
            .write_bytes(command.as_bytes())
            .await
            .map_err(|e| e.timeout_context(phase))?;
        let (code, text) = self
            .reader
            .read_status()
            .await
            .map_err(|e| e.timeout_context(phase))?;
        trace_response(phase, code);
        if code != expected {
            return Err(Error::from_status(code, text));
        }
        let lines = if self.mode == Mode::GzipRead && self.reader.starts_zlib().await? {
            self.reader
                .read_zlib_payload()
                .await?
                .split(|byte| *byte == b'\n')
                .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
                .filter(|line| *line != b"." && !line.is_empty())
                .map(<[u8]>::to_vec)
                .collect()
        } else {
            self.reader.read_data_lines(phase).await?
        };
        Ok((text, lines))
    }

    async fn read_section_limited(
        &mut self,
        max_bytes: usize,
        phase: &'static str,
    ) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        loop {
            let line = self
                .reader
                .read_line()
                .await
                .map_err(|e| e.timeout_context(phase))?;
            let Some(data) = unstuff(&line) else {
                return Ok(out);
            };
            if out.len().saturating_add(data.len()).saturating_add(2) > max_bytes {
                return Err(Error::BodyTooLarge(max_bytes));
            }
            out.extend_from_slice(data);
            out.extend_from_slice(b"\r\n");
        }
    }
}

fn article_command(label: &str, id: Option<&ArticleRef>) -> Result<String, Error> {
    Ok(match id {
        Some(id) => format!("{label} {}\r\n", checked_ref(id)?),
        None => format!("{label}\r\n"),
    })
}

fn parse_identity(text: &str) -> Result<ArticleIdentity, Error> {
    let mut fields = text.split_whitespace();
    let number = fields
        .next()
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| invalid("article reply has no valid number"))?;
    let message_id = fields
        .next()
        .ok_or_else(|| invalid("article reply has no Message-ID"))?;
    if !message_id.starts_with('<') || !message_id.ends_with('>') || message_id.len() < 3 {
        return Err(invalid("article reply has invalid Message-ID"));
    }
    Ok(ArticleIdentity {
        number,
        message_id: message_id.into(),
    })
}

fn range_wire(target: &RangeRef) -> Result<Option<String>, Error> {
    match target {
        RangeRef::Current => Ok(None),
        RangeRef::Article(id) => checked_ref(id).map(Some),
        RangeRef::Range(range) => range.wire().map(Some),
    }
}
