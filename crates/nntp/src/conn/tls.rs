//! STARTTLS upgrade. A configured upgrade has no plaintext downgrade path.

use super::*;

impl Connection {
    pub(super) async fn start_tls(
        &mut self,
        cfg: &Config,
        connector: TlsConnector,
    ) -> Result<(), Error> {
        if self.tls_active || self.mode != Mode::Plain {
            return Err(Error::Tls(
                "TLS is already active or compression has started".into(),
            ));
        }
        if !self
            .caps
            .entries
            .iter()
            .any(|(label, _)| label.eq_ignore_ascii_case("STARTTLS"))
        {
            return Err(Error::Tls(
                "server did not advertise required STARTTLS".into(),
            ));
        }
        let (code, _, _) = self.command("STARTTLS\r\n").await?;
        if code != 382 {
            return Err(Error::Tls(format!("server refused STARTTLS ({code})")));
        }
        if !self.reader.plain.is_empty() {
            return Err(Error::Tls(
                "unexpected buffered plaintext after STARTTLS".into(),
            ));
        }
        let (placeholder_read, placeholder_write) = tokio::io::split(NetStream::Disconnected);
        let reader = std::mem::replace(&mut self.reader.io, placeholder_read);
        let writer = std::mem::replace(&mut self.writer.io, placeholder_write);
        let NetStream::Tcp(tcp) = reader.unsplit(writer) else {
            return Err(Error::Tls("STARTTLS requires an unencrypted socket".into()));
        };
        let name = ServerName::try_from(cfg.host.as_str())
            .map_err(|error| Error::Tls(error.to_string()))?
            .to_owned();
        let _read = self.reader.connection.as_ref().map(|state| state.reading());
        let _write = self.writer.connection.as_ref().map(|state| state.writing());
        let tls = tokio::time::timeout(cfg.timeout, connector.connect(name, tcp))
            .await
            .map_err(|_| Error::TimeoutAt("STARTTLS handshake"))?
            .map_err(|error| Error::Tls(error.to_string()))?;
        let (reader, writer) = tokio::io::split(NetStream::Tls(Box::new(tls)));
        self.reader.io = reader;
        self.writer.io = writer;
        self.tls_active = true;
        // RFC 4642 resets protocol knowledge, with no second greeting.
        self.caps = Capabilities::empty();
        self.overview = None;
        self.strategy = HeaderStrategy::Unknown;
        self.legacy_overview = false;
        Ok(())
    }
}
