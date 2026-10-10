//! One NNTP connection. Header compression is negotiated here and never applied
//! to a connection the pool uses for article bodies.

use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use flate2::{Compress, Compression, Decompress};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::codec::{
    join_lines, parse_capabilities, parse_group_info, parse_group_status, parse_overview_line,
    parse_status_line, pop_line, unstuff, OverviewFmt,
};
use crate::compress::{compress_sync, inflate_pending, GzipReader};
use crate::error::Error;
use crate::inflate::inflate_auto;
use crate::types::{
    format_ref, ArticleRef, Capabilities, Config, GroupInfo, GroupStatus, HeaderStrategy,
    OverviewLine, TlsMode,
};
use crate::yenc;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Deflate,
    GzipRead,
}

mod auth;
mod body;
mod overview;
mod reader;
mod tls;
mod transport;

pub use reader::{
    ActiveTime, Article, ArticleIdentity, ArticleRange, DistributionPattern, GroupDescription,
    HeaderBytes, HeaderScope, HeaderValue, ListGroup, NewsDateTime, RangeRef,
};

use transport::{tls_connector, write_all_cmds, Inflate, NetStream, Reader, Writer};

/// A connected, authenticated reader session.
pub struct Connection {
    reader: Reader,
    writer: Writer,
    caps: Capabilities,
    overview: Option<OverviewFmt>,
    mode: Mode,
    strategy: HeaderStrategy,
    legacy_overview: bool,
    tls_active: bool,
    socket_metrics: Option<crate::pool::metrics::Socket>,
    registration: Option<crate::connection_stats::Registration>,
}

impl Connection {
    pub async fn connect(cfg: &Config) -> Result<Self, Error> {
        Self::connect_with_connector(cfg, tls_connector()?, None, None).await
    }

    pub(crate) async fn connect_tracked(
        cfg: &Config,
        registration: crate::connection_stats::Registration,
    ) -> Result<Self, Error> {
        let traffic = registration.state.traffic.clone();
        Self::connect_with_connector(cfg, tls_connector()?, Some(traffic), Some(registration)).await
    }

    /// Connect using an explicit trust/client-certificate policy. Defaults use
    /// public roots and validate the configured hostname.
    pub async fn connect_with_tls_config(
        cfg: &Config,
        config: Arc<rustls::ClientConfig>,
    ) -> Result<Self, Error> {
        Self::connect_with_connector(cfg, TlsConnector::from(config), None, None).await
    }

    async fn connect_with_connector(
        cfg: &Config,
        connector: TlsConnector,
        traffic: Option<Arc<crate::Traffic>>,
        registration: Option<crate::connection_stats::Registration>,
    ) -> Result<Self, Error> {
        let state = registration
            .as_ref()
            .map(|registration| registration.state.clone());
        let addr = (cfg.host.as_str(), cfg.port);
        let tcp = match tokio::time::timeout(cfg.timeout, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => return Err(Error::Connect(err.to_string())),
            Err(_) => return Err(Error::Timeout),
        };
        tcp.set_nodelay(true)
            .map_err(|err| Error::Connect(err.to_string()))?;
        let stream = match cfg.tls {
            TlsMode::Plain | TlsMode::StartTls => NetStream::Tcp(tcp),
            TlsMode::Implicit => {
                let _read = state.as_ref().map(|state| state.reading());
                let _write = state.as_ref().map(|state| state.writing());
                let name = ServerName::try_from(cfg.host.as_str())
                    .map_err(|err| Error::Tls(err.to_string()))?
                    .to_owned();
                let tls = tokio::time::timeout(cfg.timeout, connector.connect(name, tcp))
                    .await
                    .map_err(|_| Error::TimeoutAt("TLS handshake"))?
                    .map_err(|err| Error::Tls(err.to_string()))?;
                NetStream::Tls(Box::new(tls))
            }
        };
        let (io_r, io_w) = tokio::io::split(stream);
        let mut conn = Connection {
            reader: Reader {
                io: io_r,
                plain: Vec::new(),
                inflate: Inflate::None,
                timeout: cfg.timeout,
                traffic: traffic.clone(),
                connection: state.clone(),
            },
            writer: Writer {
                io: io_w,
                deflate: None,
                timeout: cfg.timeout,
                traffic,
                connection: state,
            },
            caps: Capabilities::empty(),
            overview: None,
            mode: Mode::Plain,
            strategy: HeaderStrategy::Unknown,
            legacy_overview: false,
            tls_active: cfg.tls == TlsMode::Implicit,
            socket_metrics: None,
            registration,
        };
        conn.handshake(cfg, connector).await?;
        if let Some(registration) = &conn.registration {
            registration.state.authenticate();
        }
        Ok(conn)
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    pub(crate) fn measure_socket(&mut self, metrics: Arc<crate::pool::metrics::Metrics>) {
        self.socket_metrics = Some(metrics.socket());
    }

    pub fn header_strategy(&self) -> HeaderStrategy {
        self.strategy
    }

    pub async fn quit(&mut self) -> Result<(), Error> {
        let (code, text, _) = self.command("QUIT\r\n").await?;
        if code == 205 {
            Ok(())
        } else {
            Err(Error::from_status(code, text))
        }
    }

    pub async fn date(&mut self) -> Result<String, Error> {
        let (code, text, _) = self.command("DATE\r\n").await?;
        if code == 111 {
            Ok(text)
        } else {
            Err(Error::from_status(code, text))
        }
    }

    pub async fn group(&mut self, name: &str) -> Result<GroupStatus, Error> {
        reject_ctl(name)?;
        if name.is_empty() || name.bytes().any(|byte| byte <= b' ' || byte == 127) {
            return Err(Error::Protocol {
                code: 0,
                text: "group must be a nonempty token".into(),
            });
        }
        let (code, text, _) = self.command(&format!("GROUP {name}\r\n")).await?;
        if code != 211 {
            return Err(Error::from_status(code, text));
        }
        parse_group_status(&text, name)
    }

    pub async fn list_active(&mut self, wildmat: &str) -> Result<Vec<GroupInfo>, Error> {
        let mut groups = Vec::new();
        self.list_active_each(wildmat, |group| groups.push(group))
            .await?;
        Ok(groups)
    }

    /// Stream groups without retaining the complete server list. Malformed
    /// entries are ignored; transport and command failures still fail the call.
    pub async fn list_active_each<F>(&mut self, wildmat: &str, mut on_group: F) -> Result<(), Error>
    where
        F: FnMut(GroupInfo) + Send,
    {
        reject_ctl(wildmat)?;
        trace_request("LIST ACTIVE");
        self.writer
            .write_bytes(format!("LIST ACTIVE {wildmat}\r\n").as_bytes())
            .await
            .map_err(|err| err.timeout_context("LIST ACTIVE"))?;
        let (code, text) = self
            .reader
            .read_status()
            .await
            .map_err(|err| err.timeout_context("LIST ACTIVE"))?;
        trace_response("LIST ACTIVE", code);
        if code != 215 {
            return Err(Error::from_status(code, text));
        }
        if self.mode == Mode::GzipRead
            && self
                .reader
                .starts_zlib()
                .await
                .map_err(|err| err.timeout_context("LIST ACTIVE"))?
        {
            let bytes = self
                .reader
                .read_zlib_payload()
                .await
                .map_err(|err| err.timeout_context("LIST ACTIVE"))?;
            for line in bytes.split(|byte| *byte == b'\n') {
                if let Ok(group) = parse_group_info(line) {
                    on_group(group);
                }
            }
            return Ok(());
        }
        loop {
            let line = self.reader.read_line().await?;
            match unstuff(&line) {
                None => return Ok(()),
                Some(data) => {
                    if let Ok(group) = parse_group_info(data) {
                        on_group(group);
                    }
                }
            }
        }
    }

    pub async fn stat(&mut self, id: &ArticleRef) -> Result<u64, Error> {
        Ok(self.stat_identity(Some(id)).await?.number)
    }

    pub async fn body(&mut self, id: &ArticleRef) -> Result<Vec<u8>, Error> {
        self.body_limited(id, usize::MAX).await
    }

    pub async fn head(&mut self, id: &ArticleRef) -> Result<Vec<u8>, Error> {
        let (code, text, lines) = self
            .command(&format!("HEAD {}\r\n", checked_ref(id)?))
            .await?;
        if code != 221 {
            return Err(Error::from_status(code, text));
        }
        Ok(join_lines(&lines))
    }

    /// Send every `BODY` before this future requires a response, so a server
    /// that waits for the whole window cannot stall the write half.
    pub async fn body_pipeline(
        &mut self,
        ids: &[ArticleRef],
    ) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let cmds: Vec<Vec<u8>> = ids
            .iter()
            .map(|id| checked_ref(id).map(|id| format!("BODY {id}\r\n").into_bytes()))
            .collect::<Result<_, _>>()?;
        for command in &cmds {
            crate::codec::validate_command(command)?;
        }
        let n = ids.len();
        let reader = &mut self.reader;
        let writer = &mut self.writer;
        let (write, read) = tokio::join!(write_all_cmds(writer, &cmds), reader.read_bodies(n),);
        write?;
        read
    }

    async fn command(&mut self, cmd: &str) -> Result<(u16, String, Vec<Vec<u8>>), Error> {
        let phase = command_phase(cmd);
        trace_request(phase);
        self.writer
            .write_bytes(cmd.as_bytes())
            .await
            .map_err(|err| err.timeout_context(phase))?;
        self.reader.read_response_with_phase(phase).await
    }
}

// Opt-in diagnostics include command labels and status codes only. Never print
// arguments, response text, usernames, or passwords from a provider session.
fn trace_request(phase: &'static str) {
    if std::env::var_os("NNTPBROWSER_PROTOCOL_TRACE").is_some() {
        eprintln!("NNTP {phase}: sent");
    }
}

fn trace_response(phase: &'static str, code: u16) {
    if std::env::var_os("NNTPBROWSER_PROTOCOL_TRACE").is_some() {
        eprintln!("NNTP {phase}: {code}");
    }
}

fn command_phase(cmd: &str) -> &'static str {
    let mut words = cmd.split_whitespace();
    match words.next() {
        Some("CAPABILITIES") => "CAPABILITIES",
        Some("AUTHINFO") => match words.next() {
            Some("USER") => "AUTHINFO USER",
            Some("PASS") => "AUTHINFO PASS",
            _ => "AUTHINFO",
        },
        Some("LIST") => match words.next() {
            Some("OVERVIEW.FMT") => "LIST OVERVIEW.FMT",
            Some("ACTIVE") => "LIST ACTIVE",
            _ => "LIST",
        },
        Some("XZVER") => "XZVER",
        Some("COMPRESS") => "COMPRESS DEFLATE",
        Some("XFEATURE") => "XFEATURE COMPRESS GZIP",
        Some("GROUP") => "GROUP",
        Some("MODE") => "MODE READER",
        Some("BODY") => "BODY",
        Some("HEAD") => "HEAD",
        Some("STAT") => "STAT",
        Some("DATE") => "DATE",
        Some("QUIT") => "QUIT",
        _ => "command",
    }
}

fn has_ctl(text: &str) -> bool {
    text.bytes().any(|b| b == b'\r' || b == b'\n')
}

fn checked_ref(id: &ArticleRef) -> Result<String, Error> {
    match id {
        ArticleRef::Number(0) => {
            return Err(Error::Protocol {
                code: 0,
                text: "article number must be positive".into(),
            })
        }
        ArticleRef::MessageId(message_id)
            if message_id.is_empty()
                || message_id.bytes().any(|byte| byte <= b' ' || byte == 127) =>
        {
            return Err(Error::Protocol {
                code: 0,
                text: "Message-ID must be a nonempty token".into(),
            });
        }
        _ => {}
    }
    Ok(format_ref(id))
}

fn reject_ctl(text: &str) -> Result<(), Error> {
    if has_ctl(text) {
        Err(Error::Protocol {
            code: 0,
            text: "command argument contains a line break".into(),
        })
    } else {
        Ok(())
    }
}
