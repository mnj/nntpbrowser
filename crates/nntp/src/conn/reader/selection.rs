//! Group selection and current-article navigation.
use super::*;

impl Connection {
    pub async fn next(&mut self) -> Result<ArticleIdentity, Error> {
        self.navigate("NEXT\r\n").await
    }
    pub async fn last(&mut self) -> Result<ArticleIdentity, Error> {
        self.navigate("LAST\r\n").await
    }

    async fn navigate(&mut self, command: &str) -> Result<ArticleIdentity, Error> {
        let (code, text, _) = self.command(command).await?;
        if code != 223 {
            return Err(Error::from_status(code, text));
        }
        parse_identity(&text)
    }

    /// None retains the current group. A range requires an explicit group name.
    pub async fn list_group(
        &mut self,
        group: Option<&str>,
        range: Option<ArticleRange>,
    ) -> Result<ListGroup, Error> {
        let mut articles = Vec::new();
        let group = self
            .list_group_each(group, range, |number| articles.push(number))
            .await?;
        Ok(ListGroup { group, articles })
    }

    /// Stream sparse article numbers without allocating the full retained range.
    pub async fn list_group_each<F>(
        &mut self,
        group: Option<&str>,
        range: Option<ArticleRange>,
        mut on_article: F,
    ) -> Result<GroupStatus, Error>
    where
        F: FnMut(u64) + Send,
    {
        let mut command = "LISTGROUP".to_string();
        if let Some(group) = group {
            command.push(' ');
            command.push_str(token(group)?);
        }
        if let Some(range) = range {
            if group.is_none() {
                return Err(invalid("LISTGROUP range requires a group"));
            }
            command.push(' ');
            command.push_str(&range.wire()?);
        }
        command.push_str("\r\n");
        self.writer.write_bytes(command.as_bytes()).await?;
        let (code, text) = self.reader.read_status().await?;
        if code != 211 {
            return Err(Error::from_status(code, text));
        }
        let status = parse_group_status(&text, group.unwrap_or(""))?;
        if self.mode == Mode::GzipRead && self.reader.starts_zlib().await? {
            let bytes = self.reader.read_zlib_payload().await?;
            for line in bytes.split(|byte| *byte == b'\n') {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line == b"." || line.is_empty() {
                    continue;
                }
                on_article(parse_number(line)?);
            }
        } else {
            loop {
                let line = self.reader.read_line().await?;
                let Some(line) = unstuff(&line) else { break };
                on_article(parse_number(line)?);
            }
        }
        Ok(status)
    }
}

fn parse_number(line: &[u8]) -> Result<u64, Error> {
    let n = std::str::from_utf8(line)
        .ok()
        .and_then(|text| text.parse().ok());
    n.filter(|&n| n > 0)
        .ok_or_else(|| invalid("LISTGROUP returned an invalid article number"))
}
