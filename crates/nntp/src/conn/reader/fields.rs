//! Header and overview access, including legacy reader fallbacks.
use super::*;

impl Connection {
    /// Header name excludes its colon; metadata names include their leading colon.
    /// This display helper replaces invalid UTF-8; use hdr_bytes for lossless values.
    pub async fn hdr(&mut self, field: &str, target: &RangeRef) -> Result<Vec<HeaderValue>, Error> {
        Ok(self
            .hdr_bytes(field, target)
            .await?
            .into_iter()
            .map(|header| HeaderValue {
                article: header.article,
                value: String::from_utf8_lossy(&header.value).into_owned(),
            })
            .collect())
    }

    /// Retrieve HDR/XHDR values without replacing or decoding any header octets.
    pub async fn hdr_bytes(
        &mut self,
        field: &str,
        target: &RangeRef,
    ) -> Result<Vec<HeaderBytes>, Error> {
        token(field)?;
        let mut label = "HDR";
        let legacy =
            self.caps.supports("XHDR") && !self.caps.supports("HDR") && !field.starts_with(':');
        if legacy {
            label = "XHDR";
        }
        let command = field_command(label, field, target)?;
        let reply = self
            .reader_data(&command, if legacy { 221 } else { 225 })
            .await;
        let lines = match reply {
            Ok((_, lines)) => lines,
            Err(Error::Unsupported) if label == "HDR" && !field.starts_with(':') => {
                self.reader_data(&field_command("XHDR", field, target)?, 221)
                    .await?
                    .1
            }
            Err(error) => return Err(error),
        };
        lines
            .iter()
            .map(|line| {
                let separator = line.iter().position(|&byte| byte == b' ');
                let (number, value) = match separator {
                    Some(index) => (&line[..index], &line[index + 1..]),
                    None => (line.as_slice(), b"".as_slice()),
                };
                let article = std::str::from_utf8(number)
                    .map_err(|_| invalid("HDR returned an invalid article number"))?
                    .parse()
                    .map_err(|_| invalid("HDR returned an invalid article number"))?;
                Ok(HeaderBytes {
                    article,
                    value: value.to_vec(),
                })
            })
            .collect()
    }

    pub async fn list_headers(&mut self, scope: HeaderScope) -> Result<Vec<String>, Error> {
        let command = match scope {
            HeaderScope::All => "LIST HEADERS\r\n",
            HeaderScope::MessageId => "LIST HEADERS MSGID\r\n",
            HeaderScope::Range => "LIST HEADERS RANGE\r\n",
        };
        Ok(self
            .reader_data(command, 215)
            .await?
            .1
            .iter()
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect())
    }

    /// Preserve complete overview fields, including optional fields such as Xref.
    pub async fn list_overview_format(&mut self) -> Result<Vec<String>, Error> {
        Ok(self
            .reader_data("LIST OVERVIEW.FMT\r\n", 215)
            .await?
            .1
            .iter()
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect())
    }

    /// All three RFC forms: current article, Message-ID, and a closed/open range.
    /// The streaming fetch_overview API remains preferable for huge groups.
    pub async fn over(&mut self, target: &RangeRef) -> Result<Vec<OverviewLine>, Error> {
        range_wire(target)?;
        self.ensure_overview_fmt().await?;
        let lines = self.over_lines(target).await?;
        let fmt = self.overview.as_ref().expect("overview format loaded");
        lines
            .iter()
            .map(|line| parse_overview_line(line, fmt))
            .collect()
    }

    /// Display fields in their original tab-separated order. Invalid UTF-8 is
    /// replaced; over_bytes retains the exact original octets instead.
    pub async fn over_raw(&mut self, target: &RangeRef) -> Result<Vec<Vec<String>>, Error> {
        Ok(self
            .over_lines(target)
            .await?
            .iter()
            .map(|line| {
                String::from_utf8_lossy(line)
                    .split('\t')
                    .map(str::to_owned)
                    .collect()
            })
            .collect())
    }

    /// Lossless, dot-unstuffed overview lines, including every optional field,
    /// tab separator and empty column. Lines exclude their terminating CRLF.
    pub async fn over_bytes(&mut self, target: &RangeRef) -> Result<Vec<Vec<u8>>, Error> {
        self.over_lines(target).await
    }

    async fn over_lines(&mut self, target: &RangeRef) -> Result<Vec<Vec<u8>>, Error> {
        let argument = range_wire(target)?;
        let command = |label: &str| match &argument {
            Some(value) => format!("{label} {value}\r\n"),
            None => format!("{label}\r\n"),
        };
        let label = if self.legacy_overview {
            "XOVER"
        } else {
            "OVER"
        };
        let lines = match self.reader_data(&command(label), 224).await {
            Ok((_, lines)) => lines,
            Err(Error::Unsupported) if label == "OVER" => {
                self.legacy_overview = true;
                self.reader_data(&command("XOVER"), 224).await?.1
            }
            Err(error) => return Err(error),
        };
        Ok(lines)
    }
}

fn field_command(label: &str, field: &str, target: &RangeRef) -> Result<String, Error> {
    Ok(match range_wire(target)? {
        Some(value) => format!("{label} {field} {value}\r\n"),
        None => format!("{label} {field}\r\n"),
    })
}
