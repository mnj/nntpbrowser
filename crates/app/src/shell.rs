//! Window state. The worker thread owns the network and the databases.
//!
//! The main window is a server/group sidebar plus an article browser. Server host,
//! port, TLS, and login live in the Settings dialog.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, FontId, TextureHandle};
use engine::{
    Command, Event, FileRow, GroupRow, Handle, JobRow, ListedGroup, NewServer, PostRow, ServerRow,
    TransferRate,
};

use crate::format::{
    age, format_bytes, format_stamp, grid_columns, passes_filter, scrub, FeedFilter,
};
use crate::theme;

mod actions;
mod articles;
mod connections;
mod details;
mod errors;
mod events;
mod feed;
#[cfg(feature = "ui-preview")]
mod fixture;
mod gallery;
mod group_layouts;
mod headers;
mod library;
mod library_cache;
mod lightbox;
mod media;
mod navigation;
mod options;
mod preferences;
mod process_stats;
mod queue;
mod sidebar;
mod status;
mod storage_location;
mod subscriptions;
#[cfg(test)]
mod tests;
mod textures;
mod traffic;
mod unsubscribe;
mod widgets;

use subscriptions::finish_sample;
use widgets::*;

const TEXTURE_CAP: usize = 200;
const MEDIA_LIMIT: u64 = 10 * 1024 * 1024;

struct ArticleContent {
    text: String,
    binary: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Browse,
    Queue,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowseView {
    Articles,
    Gallery,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderScope {
    Latest,
    New,
    All,
}

struct Form {
    name: String,
    host: String,
    port: String,
    tls: bool,
    starttls: bool,
    username: String,
    password: String,
    priority: String,
    connections: String,
    pipeline: String,
    fill: bool,
    enabled: bool,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: "563".into(),
            tls: true,
            starttls: false,
            username: String::new(),
            password: String::new(),
            priority: "1".into(),
            connections: "8".into(),
            pipeline: "8".into(),
            fill: false,
            enabled: true,
        }
    }
}

struct Pixels {
    key: String,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone)]
struct LibItem {
    job_id: i64,
    path: PathBuf,
    title: String,
    kind: LibKind,
    size: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibKind {
    Image,
    Video,
    Packed,
    File,
}

struct SampleSet {
    groups: Vec<ListedGroup>,
    received: u64,
    articles: u64,
    done: bool,
    started: Instant,
}

struct Columns {
    date: f32,
    title: f32,
    size: f32,
    files: f32,
    parts: f32,
    poster: f32,
    status: f32,
}

#[derive(Clone, PartialEq, Eq)]
struct PostRequest {
    group_id: i64,
    search: String,
    after: Option<(i64, i64)>,
    from: Option<i64>,
    until: Option<i64>,
}

struct HeaderProgress {
    received: u64,
    stored: u64,
    first: u64,
    last: u64,
    completed: u64,
    total: u64,
    started: Instant,
}

impl Default for HeaderProgress {
    fn default() -> Self {
        Self {
            received: 0,
            stored: 0,
            first: 0,
            last: 0,
            completed: 0,
            total: 0,
            started: Instant::now(),
        }
    }
}

pub struct Browser {
    handle: Handle,
    preferences: preferences::Preferences,
    preferences_applied: preferences::Preferences,
    #[cfg(feature = "ui-preview")]
    preview_fixture: bool,
    #[cfg(feature = "ui-preview")]
    preview_speed_tooltip: bool,
    #[cfg(feature = "ui-preview")]
    preview_connections_tooltip: bool,
    screen: Screen,
    view: BrowseView,
    filter: FeedFilter,
    servers: Vec<ServerRow>,
    groups: Vec<GroupRow>,
    posts: Vec<PostRow>,
    feed_indexes: Arc<Vec<usize>>,
    feed_dirty: bool,
    files: Vec<FileRow>,
    jobs: Vec<JobRow>,
    library: library_cache::LibraryCache,
    samples: HashMap<i64, SampleSet>,
    notice: String,
    error_history: VecDeque<errors::ErrorEntry>,
    error_count: usize,
    errors_open: bool,
    selected_server: Option<i64>,
    selected_group: Option<i64>,
    open_post: Option<i64>,
    preview_post: Option<i64>,
    article_content: Option<ArticleContent>,
    article_requested: Option<i64>,
    article_loading: bool,
    article_error: Option<String>,
    media_paths: HashMap<i64, PathBuf>,
    media_memory: HashMap<i64, media::MemoryPreview>,
    media_order: VecDeque<i64>,
    media_evicted: HashSet<i64>,
    media_memory_bytes: usize,
    media_version: u64,
    media_generation: u64,
    media_viewport: Option<(u64, Vec<i64>)>,
    media_progress: HashMap<i64, (u64, u64)>,
    media_saving: HashSet<i64>,
    media_pending: HashSet<i64>,
    media_errors: HashMap<i64, String>,
    gallery_auto: bool,
    gallery_dates: gallery::dates::DateFilter,
    search: String,
    selection: HashSet<i64>,
    anchor: Option<i64>,
    article_split: f32,
    posts_loading: bool,
    append_posts: bool,
    posts_full: bool,
    post_request: Option<PostRequest>,
    options_open: bool,
    groups_open: bool,
    picker_server: Option<i64>,
    picker_filter: String,
    picker_name: String,
    picker_loading: bool,
    picker_indexes: Vec<usize>,
    group_layouts_open: bool,
    picker_applied_filter: String,
    picker_dirty: bool,
    picker_sort: subscriptions::sort::PickerSort,
    sidebar_open: bool,
    header_limit: u64,
    header_scope: HeaderScope,
    header_progress: Option<HeaderProgress>,
    header_task_group: Option<i64>,
    header_cancel_pending: bool,
    download_limit: u64,
    confirm_delete: bool,
    select_new: bool,
    editing: Option<i64>,
    form: Form,
    known_servers: HashSet<i64>,
    group_fetches: VecDeque<i64>,
    group_fetching: Option<i64>,
    group_fetch_at: Option<Instant>,
    nzb_path: String,
    thumb_paths: HashMap<i64, PathBuf>,
    textures: HashMap<String, TextureHandle>,
    texture_order: VecDeque<String>,
    decoding: HashSet<String>,
    decode_refresh: HashMap<String, PathBuf>,
    decode_failed: HashSet<String>,
    decode_invalidated: HashSet<String>,
    pixel_tx: Sender<Pixels>,
    pixel_rx: Receiver<Pixels>,
    speeds: HashMap<i64, TransferRate>,
    network_stats: HashMap<(i64, u64), (usize, usize, f64)>,
    connections: HashMap<(i64, u64), Vec<engine::ConnectionStats>>,
    traffic: traffic::TrafficTotals,
    process_stats: process_stats::ProcessStats,
    overlay: Option<usize>,
}

impl Browser {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        let mut browser = Self::with_handle(Handle::start(default_data_dir()));
        browser.process_stats = process_stats::ProcessStats::start();
        browser
    }

    fn with_handle(handle: Handle) -> Self {
        let (pixel_tx, pixel_rx) = mpsc::channel();
        let (preferences, preference_notice) = preferences::Preferences::load(handle.data_dir());
        handle.send(Command::ConfigureNetwork {
            timeout_secs: preferences.nntp_timeout_secs,
        });
        if !preferences.downloads_dir.trim().is_empty() {
            handle.send(Command::ConfigureStorage {
                downloads_dir: PathBuf::from(preferences.downloads_dir.trim()),
            });
        }
        let initial_view = if preferences.default_gallery {
            BrowseView::Gallery
        } else {
            BrowseView::Articles
        };
        let gallery_auto = preferences.gallery_auto;
        let sidebar_open = preferences.sidebar_open;
        Self {
            handle,
            preferences_applied: preferences.clone(),
            preferences,
            #[cfg(feature = "ui-preview")]
            preview_fixture: false,
            #[cfg(feature = "ui-preview")]
            preview_speed_tooltip: false,
            #[cfg(feature = "ui-preview")]
            preview_connections_tooltip: false,
            screen: Screen::Browse,
            view: initial_view,
            filter: if initial_view == BrowseView::Gallery {
                FeedFilter::Media
            } else {
                FeedFilter::All
            },
            servers: Vec::new(),
            groups: Vec::new(),
            posts: Vec::new(),
            feed_indexes: Arc::new(Vec::new()),
            feed_dirty: true,
            files: Vec::new(),
            jobs: Vec::new(),
            library: library_cache::LibraryCache::default(),
            samples: HashMap::new(),
            notice: preference_notice.unwrap_or_default(),
            error_history: VecDeque::new(),
            error_count: 0,
            errors_open: false,
            selected_server: None,
            selected_group: None,
            open_post: None,
            preview_post: None,
            article_content: None,
            article_requested: None,
            article_loading: false,
            article_error: None,
            media_paths: HashMap::new(),
            media_memory: HashMap::new(),
            media_order: VecDeque::new(),
            media_evicted: HashSet::new(),
            media_memory_bytes: 0,
            media_version: 0,
            media_generation: 0,
            media_viewport: None,
            media_progress: HashMap::new(),
            media_saving: HashSet::new(),
            media_pending: HashSet::new(),
            media_errors: HashMap::new(),
            gallery_auto,
            gallery_dates: gallery::dates::DateFilter::default(),
            search: String::new(),
            selection: HashSet::new(),
            anchor: None,
            article_split: 0.56,
            posts_loading: false,
            append_posts: false,
            posts_full: false,
            post_request: None,
            options_open: false,
            groups_open: false,
            picker_server: None,
            picker_filter: String::new(),
            picker_name: String::new(),
            picker_loading: false,
            picker_indexes: Vec::new(),
            group_layouts_open: false,
            picker_applied_filter: String::new(),
            picker_dirty: true,
            picker_sort: subscriptions::sort::PickerSort::default(),
            sidebar_open,
            header_limit: 10_000,
            header_scope: HeaderScope::Latest,
            header_progress: None,
            header_task_group: None,
            header_cancel_pending: false,
            download_limit: 100,
            confirm_delete: false,
            select_new: false,
            editing: None,
            form: Form::default(),
            known_servers: HashSet::new(),
            group_fetches: VecDeque::new(),
            group_fetching: None,
            group_fetch_at: None,
            nzb_path: String::new(),
            thumb_paths: HashMap::new(),
            textures: HashMap::new(),
            texture_order: VecDeque::new(),
            decoding: HashSet::new(),
            decode_refresh: HashMap::new(),
            decode_failed: HashSet::new(),
            decode_invalidated: HashSet::new(),
            pixel_tx,
            pixel_rx,
            speeds: HashMap::new(),
            network_stats: HashMap::new(),
            connections: HashMap::new(),
            traffic: traffic::TrafficTotals::default(),
            process_stats: process_stats::ProcessStats::default(),
            overlay: None,
        }
    }
}

impl eframe::App for Browser {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_zoom_factor(self.preferences_applied.ui_scale);
        self.pump(ui.ctx());
        ui.ctx().request_repaint_after(Duration::from_millis(80));
        if self.preview_post.is_some() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.preview_post = None;
        }
        egui::Panel::top("toolbar")
            .exact_size(64.0)
            .resizable(false)
            .frame(theme::bar())
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .exact_size(48.0)
            .resizable(false)
            .frame(theme::bar())
            .show(ui, |ui| self.status(ui));
        if self.sidebar_open {
            egui::Panel::left("groups")
                .resizable(true)
                .default_size(300.0)
                .min_size(220.0)
                .max_size(520.0)
                .frame(theme::side())
                .show(ui, |ui| self.tree(ui));
        } else {
            egui::Panel::left("groups-minimized")
                .resizable(false)
                .exact_size(60.0)
                .frame(theme::side())
                .show(ui, |ui| self.minimized_tree(ui));
        }
        egui::CentralPanel::default()
            .frame(theme::content())
            .show(ui, |ui| self.content(ui));
        self.lightbox(ui.ctx());
        if self.preview_post.is_none()
            && (self.screen != Screen::Browse || self.view != BrowseView::Gallery)
        {
            self.request_media(&[]);
        }
        self.errors_window(ui.ctx());
        self.groups_window(ui.ctx());
        self.group_layouts_window(ui.ctx());
        self.options_window(ui.ctx());
    }
}

pub(crate) fn default_data_dir() -> PathBuf {
    if let Some(path) = storage_location::configured() {
        return path;
    }
    platform_data_dir()
}

fn platform_data_dir() -> PathBuf {
    directories::ProjectDirs::from("org", "nntpbrowser", "NNTPBrowser")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("nntpbrowser-data"))
}

pub(crate) fn configure_handle(handle: &Handle) {
    let (preferences, _) = preferences::Preferences::load(handle.data_dir());
    handle.send(Command::ConfigureNetwork {
        timeout_secs: preferences.nntp_timeout_secs,
    });
    if !preferences.downloads_dir.trim().is_empty() {
        handle.send(Command::ConfigureStorage {
            downloads_dir: PathBuf::from(preferences.downloads_dir.trim()),
        });
    }
}

pub(crate) fn downloads_dir(handle: &Handle) -> PathBuf {
    let (preferences, _) = preferences::Preferences::load(handle.data_dir());
    if preferences.downloads_dir.trim().is_empty() {
        handle.data_dir().join("downloads")
    } else {
        PathBuf::from(preferences.downloads_dir.trim())
    }
}
