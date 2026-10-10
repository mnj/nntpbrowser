//! Overview negotiation and streamed/plain or compressed header responses.

use super::*;

impl Connection {
    pub async fn fetch_overview<F>(
        &mut self,
        first: u64,
        last: u64,
        hint: HeaderStrategy,
        mut on_line: F,
    ) -> Result<HeaderStrategy, Error>
    where
        F: FnMut(OverviewLine) + Send,
    {
        self.ensure_overview_fmt().await?;
        let strategy = match hint {
            HeaderStrategy::Unknown => self.negotiate(first, last, &mut on_line).await?,
            HeaderStrategy::Xzver => {
                self.overview_xzver(first, last, &mut on_line).await?;
                HeaderStrategy::Xzver
            }
            HeaderStrategy::CompressDeflate
                if self.mode != Mode::Deflate && !self.caps.compress_deflate =>
            {
                // A previous session's strategy is only a hint. RFC 8054
                // requires COMPRESS to be advertised by the current session.
                self.negotiate(first, last, &mut on_line).await?
            }
            HeaderStrategy::CompressDeflate => match self.ensure_compress_deflate().await {
                Ok(()) => {
                    self.overview_over(first, last, &mut on_line).await?;
                    HeaderStrategy::CompressDeflate
                }
                Err(Error::Unsupported)
                | Err(Error::Protocol {
                    code: 403 | 502 | 503,
                    ..
                }) => {
                    self.negotiate_without_deflate(first, last, &mut on_line)
                        .await?
                }
                Err(error) => return Err(error),
            },
            HeaderStrategy::XfeatureGzip => {
                self.ensure_xfeature_gzip().await?;
                self.overview_over(first, last, &mut on_line).await?;
                HeaderStrategy::XfeatureGzip
            }
            HeaderStrategy::Plain => {
                self.overview_over(first, last, &mut on_line).await?;
                HeaderStrategy::Plain
            }
        };
        self.strategy = strategy;
        Ok(strategy)
    }

    pub(super) async fn ensure_overview_fmt(&mut self) -> Result<(), Error> {
        if self.overview.is_some() {
            return Ok(());
        }
        let (code, text, lines) = self.command("LIST OVERVIEW.FMT\r\n").await?;
        if code == 215 {
            let names = lines
                .iter()
                .map(|line| String::from_utf8_lossy(line).into_owned())
                .collect::<Vec<_>>();
            self.overview = Some(OverviewFmt::from_header_names(&names));
            Ok(())
        } else if code == 500 || code == 501 {
            self.overview = Some(OverviewFmt::positional());
            Ok(())
        } else {
            Err(Error::from_status(code, text))
        }
    }

    async fn negotiate<F>(
        &mut self,
        first: u64,
        last: u64,
        on_line: &mut F,
    ) -> Result<HeaderStrategy, Error>
    where
        F: FnMut(OverviewLine) + Send,
    {
        match self.overview_xzver(first, last, on_line).await {
            Ok(()) => return Ok(HeaderStrategy::Xzver),
            Err(Error::Unsupported) => {}
            Err(err) => return Err(err),
        }
        if self.caps.compress_deflate {
            match self.ensure_compress_deflate().await {
                Ok(()) => {
                    self.overview_over(first, last, on_line).await?;
                    return Ok(HeaderStrategy::CompressDeflate);
                }
                Err(Error::Unsupported)
                | Err(Error::Protocol {
                    code: 403 | 502 | 503,
                    ..
                }) => {}
                Err(err) => return Err(err),
            }
        }
        self.negotiate_without_deflate(first, last, on_line).await
    }

    async fn negotiate_without_deflate<F>(
        &mut self,
        first: u64,
        last: u64,
        on_line: &mut F,
    ) -> Result<HeaderStrategy, Error>
    where
        F: FnMut(OverviewLine) + Send,
    {
        match self.ensure_xfeature_gzip().await {
            Ok(()) => {
                self.overview_over(first, last, on_line).await?;
                return Ok(HeaderStrategy::XfeatureGzip);
            }
            Err(Error::Unsupported)
            | Err(Error::Protocol {
                code: 403 | 502, ..
            }) => {}
            Err(err) => return Err(err),
        }
        self.overview_over(first, last, on_line).await?;
        Ok(HeaderStrategy::Plain)
    }

    async fn ensure_compress_deflate(&mut self) -> Result<(), Error> {
        if self.mode == Mode::Deflate {
            return Ok(());
        }
        if self.mode != Mode::Plain {
            return Err(Error::Compression(
                "connection already uses another compression".into(),
            ));
        }
        if !self.caps.compress_deflate {
            return Err(Error::Unsupported);
        }
        let (code, text, _) = self.command("COMPRESS DEFLATE\r\n").await?;
        if code != 206 {
            return Err(Error::from_status(code, text));
        }
        self.reader.enable_raw()?;
        self.writer.enable_deflate();
        self.mode = Mode::Deflate;
        Ok(())
    }

    async fn ensure_xfeature_gzip(&mut self) -> Result<(), Error> {
        if self.mode == Mode::GzipRead {
            return Ok(());
        }
        if self.mode != Mode::Plain {
            return Err(Error::Compression(
                "connection already uses another compression".into(),
            ));
        }
        let (code, text, _) = self.command("XFEATURE COMPRESS GZIP\r\n").await?;
        if code != 290 {
            return Err(Error::from_status(code, text));
        }
        self.reader.enable_gzip()?;
        self.mode = Mode::GzipRead;
        Ok(())
    }

    async fn overview_xzver<F>(
        &mut self,
        first: u64,
        last: u64,
        on_line: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(OverviewLine),
    {
        trace_request("XZVER");
        self.writer
            .write_bytes(format!("XZVER {first}-{last}\r\n").as_bytes())
            .await
            .map_err(|err| err.timeout_context("XZVER"))?;
        let (code, text) = self
            .reader
            .read_status()
            .await
            .map_err(|err| err.timeout_context("XZVER"))?;
        trace_response("XZVER", code);
        if code != 224 {
            return Err(Error::from_status(code, text));
        }
        if self
            .reader
            .starts_zlib()
            .await
            .map_err(|err| err.timeout_context("XZVER"))?
        {
            let inflated = self
                .reader
                .read_zlib_payload()
                .await
                .map_err(|err| err.timeout_context("XZVER"))?;
            return self.emit_overview_text(&inflated, on_line);
        }
        let lines = self.reader.read_encoded_data_lines("XZVER").await?;
        let payload = yenc::decode_lines(&lines)?;
        let inflated = inflate_auto(&payload.data)?;
        self.reader.measure_decoded(inflated.len());
        self.emit_overview_text(&inflated, on_line)
    }

    async fn overview_over<F>(
        &mut self,
        first: u64,
        last: u64,
        on_line: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(OverviewLine),
    {
        let mut phase = if self.legacy_overview {
            "XOVER"
        } else {
            "OVER"
        };
        let (mut code, mut text) = self.send_overview(phase, first, last).await?;
        if matches!(code, 500 | 501) && phase == "OVER" {
            self.legacy_overview = true;
            phase = "XOVER";
            (code, text) = self.send_overview(phase, first, last).await?;
        }
        if code != 224 {
            if code == 500 || code == 501 {
                return Err(Error::OverviewUnsupported);
            }
            return Err(Error::from_status(code, text));
        }
        if self.mode == Mode::GzipRead
            && self
                .reader
                .starts_zlib()
                .await
                .map_err(|err| err.timeout_context(phase))?
        {
            let inflated = self
                .reader
                .read_zlib_payload()
                .await
                .map_err(|err| err.timeout_context(phase))?;
            return self.emit_overview_text(&inflated, on_line);
        }
        let fmt = self.overview.clone().unwrap_or_default();
        loop {
            let line = self
                .reader
                .read_line()
                .await
                .map_err(|err| err.timeout_context(phase))?;
            match unstuff(&line) {
                None => break,
                Some([]) => {}
                Some(data) => {
                    let parsed = parse_overview_line(data, &fmt)?;
                    on_line(parsed);
                }
            }
        }
        Ok(())
    }

    async fn send_overview(
        &mut self,
        phase: &'static str,
        first: u64,
        last: u64,
    ) -> Result<(u16, String), Error> {
        trace_request(phase);
        self.writer
            .write_bytes(format!("{phase} {first}-{last}\r\n").as_bytes())
            .await
            .map_err(|err| err.timeout_context(phase))?;
        let result = self
            .reader
            .read_status()
            .await
            .map_err(|err| err.timeout_context(phase))?;
        trace_response(phase, result.0);
        Ok(result)
    }

    fn emit_overview_text<F>(&self, bytes: &[u8], on_line: &mut F) -> Result<(), Error>
    where
        F: FnMut(OverviewLine),
    {
        let fmt = self.overview.clone().unwrap_or_default();
        let text = String::from_utf8_lossy(bytes);
        for raw in text.split('\n') {
            let line = raw.trim_end_matches('\r');
            if line == "." {
                break;
            }
            if line.is_empty() {
                continue;
            }
            let parsed = parse_overview_line(line.as_bytes(), &fmt)?;
            on_line(parsed);
        }
        Ok(())
    }
}
