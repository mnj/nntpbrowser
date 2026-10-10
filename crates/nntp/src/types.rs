use std::time::Duration;

/// How to secure the socket. STARTTLS must succeed before credentials are sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsMode {
    Implicit,
    StartTls,
    Plain,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub username: Option<String>,
    pub password: Option<String>,
    pub timeout: Duration,
}

/// Remembered per server after the first header fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderStrategy {
    #[default]
    Unknown,
    Xzver,
    CompressDeflate,
    XfeatureGzip,
    Plain,
}

impl HeaderStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            HeaderStrategy::Unknown => "unknown",
            HeaderStrategy::Xzver => "xzver",
            HeaderStrategy::CompressDeflate => "compress-deflate",
            HeaderStrategy::XfeatureGzip => "xfeature-gzip",
            HeaderStrategy::Plain => "plain",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    /// Capability label, then arguments. Labels are stored as the server sent them.
    pub entries: Vec<(String, Vec<String>)>,
    pub compress_deflate: bool,
    pub over: bool,
    pub mode_reader: bool,
}

impl Capabilities {
    pub fn supports(&self, label: &str) -> bool {
        self.entries
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(label))
    }

    pub fn arguments(&self, label: &str) -> Option<&[String]> {
        self.entries
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(label))
            .map(|(_, args)| args.as_slice())
    }

    pub fn supports_list(&self, keyword: &str) -> bool {
        self.arguments("LIST")
            .is_some_and(|args| args.iter().any(|name| name.eq_ignore_ascii_case(keyword)))
    }

    pub(crate) fn empty() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
pub struct GroupInfo {
    pub name: String,
    pub high: u64,
    pub low: u64,
    pub status: char,
}

#[derive(Debug, Clone)]
pub struct GroupStatus {
    pub count: u64,
    pub low: u64,
    pub high: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewLine {
    pub article: u64,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub message_id: String,
    pub references: String,
    pub bytes: u64,
    pub lines: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArticleRef {
    Number(u64),
    /// Stored without angle brackets. The command writer adds them when missing.
    MessageId(String),
}

pub(crate) fn format_ref(id: &ArticleRef) -> String {
    match id {
        ArticleRef::Number(n) => n.to_string(),
        ArticleRef::MessageId(m) => {
            let m = m.trim();
            if m.starts_with('<') && m.ends_with('>') {
                m.to_string()
            } else {
                format!("<{m}>")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerEndpoint {
    pub id: i64,
    pub config: Config,
    pub priority: i32,
    pub fill: bool,
    pub max_connections: usize,
    pub pipeline_depth: usize,
}
