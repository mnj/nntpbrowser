use crate::grouper::FileKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// Upgrade a plain connection with STARTTLS before sending credentials.
    pub starttls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub priority: i32,
    pub max_connections: i64,
    pub pipeline_depth: i64,
    pub fill: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerRow {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub starttls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub priority: i32,
    pub max_connections: i64,
    pub pipeline_depth: i64,
    pub fill: bool,
    pub enabled: bool,
    pub header_compression: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    pub id: i64,
    pub server_id: i64,
    pub name: String,
    pub low: i64,
    pub high: i64,
    pub watermark: i64,
    pub subscribed: bool,
    pub last_refresh: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IncomingFile {
    pub name: String,
    pub size: i64,
    pub expected_segments: i64,
    pub kind: FileKind,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IncomingPost {
    pub group_key: String,
    pub title: String,
    pub poster: String,
    pub posted_at: i64,
    pub files: Vec<IncomingFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostRow {
    pub id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub title: String,
    pub poster: String,
    pub posted_at: i64,
    pub total_bytes: i64,
    pub file_count: i64,
    pub segment_count: i64,
    pub incomplete: bool,
    pub has_image: bool,
    pub has_video: bool,
    pub has_par2: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: i64,
    pub post_id: i64,
    pub name: String,
    pub size: i64,
    pub expected_segments: i64,
    pub kind: FileKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSegment {
    pub filename: String,
    pub part: i64,
    pub message_id: String,
    pub begin: i64,
    pub end: i64,
    pub article: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub id: i64,
    pub post_id: Option<i64>,
    pub name: String,
    pub dest_dir: String,
    pub dest_path: String,
    pub state: String,
    pub bytes_done: i64,
    pub bytes_total: i64,
    pub error: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRow {
    pub id: i64,
    pub job_id: i64,
    pub filename: String,
    pub part: i64,
    pub message_id: String,
    pub begin: i64,
    pub end: i64,
    pub article: Option<i64>,
    pub state: String,
    pub server_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbRow {
    pub post_id: i64,
    pub path: String,
    pub width: i64,
    pub height: i64,
}
