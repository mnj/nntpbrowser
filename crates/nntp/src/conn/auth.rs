//! TLS-first authentication and advertised SASL mechanism negotiation.

use super::*;
use crate::sasl::{self, Scram};

impl Connection {
    pub(super) async fn handshake(
        &mut self,
        cfg: &Config,
        connector: TlsConnector,
    ) -> Result<(), Error> {
        let (code, text, _) = self.reader.read_response_with_phase("greeting").await?;
        if !matches!(code, 200 | 201) {
            return Err(Error::from_status(code, text));
        }
        self.refresh_capabilities(cfg.username.is_some()).await?;
        if cfg.tls == TlsMode::StartTls {
            self.start_tls(cfg, connector).await?;
            self.refresh_capabilities(cfg.username.is_some()).await?;
        }
        if let Some(user) = &cfg.username {
            self.authenticate(user, cfg.password.as_deref()).await?;
            self.refresh_capabilities(false).await?;
        }
        // RFC 4642 forbids MODE READER after TLS is established.
        if self.caps.mode_reader && !self.tls_active {
            let (code, text, _) = self.command("MODE READER\r\n").await?;
            if !matches!(code, 200 | 201 | 500 | 501 | 502) {
                return Err(Error::from_status(code, text));
            }
            if matches!(code, 200 | 201) {
                self.refresh_capabilities(false).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn refresh_capabilities(
        &mut self,
        allow_auth_required: bool,
    ) -> Result<(), Error> {
        let (code, text, lines) = self.command("CAPABILITIES\r\n").await?;
        match code {
            101 => self.caps = parse_capabilities(&lines),
            500 | 501 => self.caps = Capabilities::empty(),
            480 if allow_auth_required => self.caps = Capabilities::empty(),
            _ => return Err(Error::from_status(code, text)),
        }
        let has = |name: &str| {
            self.caps
                .entries
                .iter()
                .any(|(label, _)| label.eq_ignore_ascii_case(name))
        };
        self.legacy_overview = has("XOVER") && !has("OVER");
        Ok(())
    }

    async fn authenticate(&mut self, user: &str, pass: Option<&str>) -> Result<(), Error> {
        if let Some(password) = pass {
            if self.advertises_sasl("SCRAM-SHA-256") {
                match self.auth_scram(user, password).await {
                    Err(Error::Unsupported) => {}
                    result => return result,
                }
            }
            if self.advertises_sasl("PLAIN") {
                match self.auth_plain(user, password).await {
                    Err(Error::Unsupported) => {}
                    result => return result,
                }
            }
        }
        self.auth_user_pass(user, pass).await
    }

    fn advertises_sasl(&self, mechanism: &str) -> bool {
        self.caps.entries.iter().any(|(label, algorithms)| {
            label.eq_ignore_ascii_case("SASL")
                && algorithms
                    .iter()
                    .any(|algorithm| algorithm.eq_ignore_ascii_case(mechanism))
        })
    }

    async fn auth_user_pass(&mut self, user: &str, pass: Option<&str>) -> Result<(), Error> {
        if has_ctl(user) || has_ctl(pass.unwrap_or("")) {
            return Err(Error::Auth);
        }
        let (code, _, _) = self
            .command(&format!("AUTHINFO USER {user}\r\n"))
            .await
            .map_err(scrub_auth_error)?;
        match code {
            281 => return Ok(()),
            381 => {}
            _ => return Err(Error::Auth),
        }
        let (code, _, _) = self
            .command(&format!("AUTHINFO PASS {}\r\n", pass.unwrap_or("")))
            .await
            .map_err(scrub_auth_error)?;
        if code == 281 {
            Ok(())
        } else {
            Err(Error::Auth)
        }
    }

    async fn auth_plain(&mut self, user: &str, password: &str) -> Result<(), Error> {
        let response = sasl::encode(&sasl::plain(user, password)?);
        let (code, text) = self
            .sasl_exchange(&format!("AUTHINFO SASL PLAIN {response}\r\n"))
            .await?;
        match code {
            281 => Ok(()),
            383 => {
                if sasl::decode(&text).map_or(true, |bytes| !bytes.is_empty()) {
                    self.cancel_sasl().await;
                    return Err(Error::Auth);
                }
                let (code, _) = self.sasl_exchange(&format!("{response}\r\n")).await?;
                if code == 281 {
                    Ok(())
                } else {
                    Err(Error::Auth)
                }
            }
            500 | 501 | 503 => Err(Error::Unsupported),
            _ => Err(Error::Auth),
        }
    }

    async fn auth_scram(&mut self, user: &str, password: &str) -> Result<(), Error> {
        let mut scram = Scram::new(user, password)?;
        let first = sasl::encode(scram.first().as_bytes());
        let (code, text) = self
            .sasl_exchange(&format!("AUTHINFO SASL SCRAM-SHA-256 {first}\r\n"))
            .await?;
        if matches!(code, 500 | 501 | 503) {
            return Err(Error::Unsupported);
        }
        if code != 383 {
            return Err(Error::Auth);
        }
        let response = sasl::decode(&text).and_then(|bytes| scram.respond(&bytes));
        let response = match response {
            Ok(response) => response,
            Err(_) => {
                self.cancel_sasl().await;
                return Err(Error::Auth);
            }
        };
        let (code, text) = self
            .sasl_exchange(&format!("{}\r\n", sasl::encode(response.as_bytes())))
            .await?;
        if !matches!(code, 283 | 383) {
            return Err(Error::Auth);
        }
        if sasl::decode(&text)
            .and_then(|bytes| scram.verify(&bytes))
            .is_err()
        {
            if code == 383 {
                self.cancel_sasl().await;
            }
            return Err(Error::Auth);
        }
        if code == 383 {
            let (code, _) = self.sasl_exchange("=\r\n").await?;
            if code != 281 {
                return Err(Error::Auth);
            }
        }
        Ok(())
    }

    async fn sasl_exchange(&mut self, command: &str) -> Result<(u16, String), Error> {
        trace_request("AUTHINFO SASL");
        self.writer
            .write_sasl_bytes(command.as_bytes())
            .await
            .map_err(|error| error.timeout_context("AUTHINFO SASL"))?;
        let (code, text) = self
            .reader
            .read_status()
            .await
            .map_err(|error| match error {
                Error::Protocol { .. } => Error::Auth,
                error => error.timeout_context("AUTHINFO SASL"),
            })?;
        trace_response("AUTHINFO SASL", code);
        if text.len() > sasl::WIRE_LIMIT {
            return Err(Error::Auth);
        }
        Ok((code, text))
    }

    async fn cancel_sasl(&mut self) {
        let _ = self.sasl_exchange("*\r\n").await;
    }
}

fn scrub_auth_error(error: Error) -> Error {
    match error {
        Error::Protocol { .. } => Error::Auth,
        error => error,
    }
}
