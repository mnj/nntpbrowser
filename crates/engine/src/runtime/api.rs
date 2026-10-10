//! Commands, events, and the frontend handle.

use super::{run_worker, set_dir_mode};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use store::{GroupRow, NewServer, ServerRow};
use tokio::sync::mpsc::UnboundedReceiver;

#[derive(Debug, Clone)]
pub enum Command {
    ConfigureStorage {
        downloads_dir: PathBuf,
    },
    ConfigureNetwork {
        timeout_secs: u64,
    },
    Refresh,
    SaveServer(NewServer),
    SaveServerEnabled {
        server: NewServer,
        enabled: bool,
    },
    UpdateServer {
        id: i64,
        server: NewServer,
        enabled: bool,
    },
    DeleteServer(i64),
    TestServer(i64),
    Subscribe {
        server_id: i64,
        name: String,
    },
    Unsubscribe {
        server_id: i64,
        name: String,
    },
    ListActive {
        server_id: i64,
        wildmat: String,
    },
    SelectGroup(i64),
    ShowGroups(i64),
    SyncGroup(i64),
    /// Cancel only a currently running header fetch for this group.
    CancelHeaderFetch(i64),
    /// Fetch the latest 10,000 headers only when this group has no cached ranges.
    EnsureGroupHeaders(i64),
    /// Import the newest `limit` article numbers, or all retained headers.
    /// Already stored ranges are skipped, allowing a later, larger backfill.
    FetchHeaders {
        group_id: i64,
        limit: Option<u64>,
    },
    LoadPosts {
        group_id: i64,
        after: Option<(i64, i64)>,
        search: String,
    },
    LoadPostsInDateRange {
        group_id: i64,
        after: Option<(i64, i64)>,
        search: String,
        from: Option<i64>,
        until: Option<i64>,
    },
    OpenPost(i64),
    QueuePost {
        post_id: i64,
    },
    QueueGroup {
        group_id: i64,
        limit: Option<u64>,
        search: String,
    },
    ImportNzb {
        xml: String,
        dest: PathBuf,
    },
    Pause(i64),
    Resume(i64),
    Cancel(i64),
    DeleteDownload {
        job_id: i64,
        remove_files: bool,
    },
    Preview(i64),
    /// Queue known-size images/videos for the Gallery, without ancillary files.
    CacheMedia {
        post_id: i64,
        max_bytes: u64,
    },
    ReadArticle(i64),
    PreviewMedia {
        post_id: i64,
        max_bytes: u64,
        generation: u64,
    },
    PreviewContext(u64),
    /// Stop queued/in-flight previews outside the current Gallery viewport.
    /// Completed previews remain available for reuse and explicit Save.
    RetainPreviewMedia {
        post_ids: Vec<i64>,
        generation: u64,
    },
    SavePreviewMedia {
        post_id: i64,
        generation: u64,
    },
}

/// One newsgroup from `LIST ACTIVE`, with the server's article range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedGroup {
    pub name: String,
    pub low: u64,
    pub high: u64,
    pub status: char,
}

impl ListedGroup {
    /// Articles in the server's current range. An empty group reports high below low.
    pub fn posts(&self) -> u64 {
        if self.low == 0 && self.high == 0 {
            0
        } else if self.high >= self.low {
            self.high.saturating_sub(self.low).saturating_add(1)
        } else {
            0
        }
    }
}

/// One live NNTP connection, with five-second average receive/send rates.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionStats {
    pub id: u64,
    pub authenticated: bool,
    pub read_pending: bool,
    pub write_pending: bool,
    /// Whether received bytes increased since the previous 250 ms sample.
    pub receiving: bool,
    /// Whether sent bytes increased since the previous 250 ms sample.
    pub sending: bool,
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub received_per_second: f64,
    pub sent_per_second: f64,
}

#[derive(Debug, Clone)]
pub enum Event {
    ConnectionStats {
        server_id: i64,
        pool_id: u64,
        connections: Vec<ConnectionStats>,
    },
    NetworkTraffic {
        server_id: i64,
        pool_id: u64,
        traffic: nntp::TrafficSnapshot,
        closed: bool,
    },
    NetworkStats {
        server_id: i64,
        pool_id: u64,
        open: usize,
        active: usize,
        bytes_per_second: f64,
    },
    Servers(Vec<ServerRow>),
    Groups(Vec<GroupRow>),
    GroupRows {
        server_id: i64,
        groups: Vec<GroupRow>,
    },
    GroupUnsubscribed {
        group_id: i64,
        post_ids: Vec<i64>,
    },
    Posts(Vec<store::PostRow>),
    PostsPage {
        group_id: i64,
        search: String,
        after: Option<(i64, i64)>,
        posts: Vec<store::PostRow>,
    },
    PostsDatePage {
        group_id: i64,
        search: String,
        after: Option<(i64, i64)>,
        from: Option<i64>,
        until: Option<i64>,
        posts: Vec<store::PostRow>,
    },
    PostsDateFailed {
        group_id: i64,
        search: String,
        after: Option<(i64, i64)>,
        from: Option<i64>,
        until: Option<i64>,
        error: String,
    },
    HeaderFetchFinished {
        group_id: i64,
        error: Option<String>,
    },
    HeaderFetchCancelled {
        group_id: i64,
    },
    HeaderProgress {
        group_id: i64,
        received: u64,
        stored: u64,
        /// Article-number slots scanned across uncached requested ranges.
        completed: u64,
        /// Inclusive length of uncached requested ranges, including sparse holes.
        total: u64,
        first: u64,
        last: u64,
    },
    PostsFailed {
        group_id: i64,
        search: String,
        after: Option<(i64, i64)>,
        error: String,
    },
    Files {
        post_id: i64,
        files: Vec<store::FileRow>,
    },
    Jobs(Vec<store::JobRow>),
    DownloadDeleted {
        job_id: i64,
        remove_files: bool,
    },
    DownloadDeleteFailed {
        job_id: i64,
        error: String,
    },
    GroupSample {
        server_id: i64,
        groups: Vec<ListedGroup>,
        received: u64,
        done: bool,
    },
    Notice(String),
    Thumb {
        post_id: i64,
        path: PathBuf,
    },
    MediaCached {
        post_id: i64,
        path: PathBuf,
    },
    MediaCacheFailed {
        post_id: i64,
        error: String,
    },
    ArticleContent {
        post_id: i64,
        text: String,
        binary: bool,
    },
    ArticleReadFailed {
        post_id: i64,
        error: String,
    },
    PreviewMediaReady {
        post_id: i64,
        generation: u64,
        name: String,
        bytes: std::sync::Arc<[u8]>,
    },
    PreviewMediaProgress {
        post_id: i64,
        generation: u64,
        received: u64,
        total: u64,
    },
    PreviewMediaFailed {
        post_id: i64,
        generation: u64,
        error: String,
    },
    MediaSaved {
        post_id: i64,
        path: PathBuf,
    },
    MediaSaveFailed {
        post_id: i64,
        error: String,
    },
}

pub struct Handle {
    cmd: crossbeam_channel::Sender<Command>,
    events: Mutex<UnboundedReceiver<Event>>,
    data_dir: PathBuf,
}

impl Handle {
    pub fn start(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let _ = std::fs::create_dir_all(&data_dir);
        set_dir_mode(&data_dir);
        let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
        let (evt_tx, evt_rx) = tokio::sync::mpsc::unbounded_channel();
        let dir = data_dir.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(run_worker(dir, cmd_rx, evt_tx));
        });
        Self {
            cmd: cmd_tx,
            events: Mutex::new(evt_rx),
            data_dir,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.cmd.send(command);
    }

    pub fn poll(&self) -> Vec<Event> {
        let mut rx = self.events.lock().expect("event lock");
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}
