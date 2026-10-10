//! Integration tests using local NNTP fixtures.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use store::{NewSegment, NewServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::UnboundedReceiver;

async fn wait_servers(handle: &Handle, inbox: &mut Vec<Event>) -> Vec<ServerRow> {
    wait_event("servers", handle, inbox, |event| match event {
        Event::Servers(servers) if !servers.is_empty() => Some(servers.clone()),
        _ => None,
    })
    .await
}

async fn wait_groups(handle: &Handle, inbox: &mut Vec<Event>) -> Vec<GroupRow> {
    wait_event("groups", handle, inbox, |event| match event {
        Event::GroupRows { groups, .. } if !groups.is_empty() => Some(groups.clone()),
        _ => None,
    })
    .await
}

async fn wait_posts(handle: &Handle, inbox: &mut Vec<Event>) -> Vec<store::PostRow> {
    wait_event("posts", handle, inbox, |event| match event {
        Event::PostsPage { posts, .. } if !posts.is_empty() => Some(posts.clone()),
        _ => None,
    })
    .await
}

async fn wait_jobs(
    handle: &Handle,
    inbox: &mut Vec<Event>,
    pred: impl Fn(&[store::JobRow]) -> bool,
) -> Vec<store::JobRow> {
    wait_event("jobs", handle, inbox, |event| match event {
        Event::Jobs(jobs) if pred(jobs) => Some(jobs.clone()),
        _ => None,
    })
    .await
}

async fn wait_notice(
    handle: &Handle,
    inbox: &mut Vec<Event>,
    pred: impl Fn(&str) -> bool,
) -> String {
    wait_event("notice", handle, inbox, |event| match event {
        Event::Notice(text) if pred(text) => Some(text.clone()),
        _ => None,
    })
    .await
}

async fn wait_event<T>(
    label: &str,
    handle: &Handle,
    inbox: &mut Vec<Event>,
    mut pred: impl FnMut(&Event) -> Option<T>,
) -> T {
    let start = std::time::Instant::now();
    let mut seen = Vec::new();
    loop {
        inbox.extend(handle.poll());
        if let Some(index) = inbox.iter().position(|event| pred(event).is_some()) {
            let event = inbox.remove(index);
            return pred(&event).expect("predicate matched");
        }
        if inbox.len() > seen.len() {
            seen = inbox.iter().map(event_tag).collect();
        }
        if start.elapsed() > Duration::from_secs(8) {
            panic!("timed out waiting for {label}. saw {seen:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn serve<F, Fut>(handler: F) -> std::net::SocketAddr
where
    F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(handler(sock));
        }
    });
    tokio::task::yield_now().await;
    addr
}

async fn read_line(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = match tokio::time::timeout(Duration::from_secs(3), sock.read(&mut byte)).await {
            Ok(Ok(0)) | Err(_) => return String::new(),
            Ok(Ok(_)) => 1,
            Ok(Err(_)) => return String::new(),
        };
        let _ = n;
        buf.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8_lossy(&buf)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

async fn write_all(sock: &mut tokio::net::TcpStream, data: &[u8]) {
    sock.write_all(data).await.unwrap();
    sock.flush().await.unwrap();
}

async fn write_multiline(sock: &mut tokio::net::TcpStream, article: &[u8]) {
    let mut out = b"222 0 <id>\r\n".to_vec();
    for line in article.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        if line.first() == Some(&b'.') {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    write_all(sock, &out).await;
}
fn event_tag(event: &Event) -> String {
    match event {
        Event::ConnectionStats {
            server_id,
            connections,
            ..
        } => format!("connections:{server_id}:{}", connections.len()),
        Event::NetworkTraffic {
            server_id, closed, ..
        } => format!("traffic:{server_id}:{closed}"),
        Event::NetworkStats {
            server_id, active, ..
        } => format!("network:{server_id}:{active}"),
        Event::DownloadDeleted { job_id, .. } => format!("download-removed:{job_id}"),
        Event::DownloadDeleteFailed { job_id, error } => {
            format!("download-remove-failed:{job_id}:{error}")
        }
        Event::Servers(servers) => format!("servers:{}", servers.len()),
        Event::GroupUnsubscribed { group_id, .. } => format!("unsubscribed:{group_id}"),
        Event::Groups(groups) | Event::GroupRows { groups, .. } => {
            format!("groups:{}", groups.len())
        }
        Event::Posts(posts)
        | Event::PostsPage { posts, .. }
        | Event::PostsDatePage { posts, .. } => format!("posts:{}", posts.len()),
        Event::Files { post_id, files } => format!("files:{post_id}:{}", files.len()),
        Event::Jobs(jobs) => format!(
            "jobs:{}",
            jobs.iter()
                .map(|job| job.state.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ),
        Event::GroupSample { groups, .. } => format!("sample:{}", groups.len()),
        Event::Notice(text) => format!("notice:{text}"),
        Event::Thumb { post_id, .. } => format!("thumb:{post_id}"),
        Event::MediaCached { post_id, .. } => format!("media:{post_id}"),
        Event::MediaCacheFailed { post_id, .. } => format!("media-failed:{post_id}"),
        Event::ArticleContent { post_id, .. } => format!("article:{post_id}"),
        Event::ArticleReadFailed { post_id, .. } => format!("article-failed:{post_id}"),
        Event::PreviewMediaReady { post_id, .. } => format!("ram-preview:{post_id}"),
        Event::PreviewMediaProgress { post_id, .. } => format!("ram-preview-progress:{post_id}"),
        Event::PreviewMediaFailed { post_id, .. } => format!("ram-preview-failed:{post_id}"),
        Event::MediaSaved { post_id, .. } => format!("media-saved:{post_id}"),
        Event::MediaSaveFailed { post_id, .. } => format!("media-save-failed:{post_id}"),
        Event::PostsFailed {
            group_id, error, ..
        }
        | Event::PostsDateFailed {
            group_id, error, ..
        } => format!("posts-failed:{group_id}:{error}"),
        Event::HeaderProgress {
            group_id,
            received,
            stored,
            ..
        } => format!("header-progress:{group_id}:{received}:{stored}"),
        Event::HeaderFetchFinished { group_id, error } => {
            format!("header-fetch:{group_id}:{error:?}")
        }
        Event::HeaderFetchCancelled { group_id } => format!("header-cancelled:{group_id}"),
    }
}

fn fixture_worker() -> (Worker, UnboundedReceiver<Event>) {
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "nntpbrowser-engine-fixture-{}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let (evt, events) = tokio::sync::mpsc::unbounded_channel();
    let (_, cmd_rx) = crossbeam_channel::unbounded();
    (
        Worker {
            catalog: Catalog::open(root.join("catalog.sqlite")).unwrap(),
            headers: HeaderStore::open(root.join("headers")).unwrap(),
            data_dir: root.clone(),
            pools: HashMap::new(),
            network_timeout_secs: 8,
            network_timeout_dirty: false,
            active_header_fetch: None,
            header_fetch_cancelled: false,
            transfers: tokio::task::JoinSet::new(),
            in_flight: HashMap::new(),
            transfer_generation: HashMap::new(),
            preview_context: 0,
            preview_tasks: tokio::task::JoinSet::new(),
            preview_active: HashMap::new(),
            preview_queue: VecDeque::new(),
            preview_wanted: None,
            preview_pending: HashSet::new(),
            preview_cache: VecDeque::new(),
            paused: HashSet::new(),
            cancelled: HashSet::new(),
            cmd_rx,
            deferred: VecDeque::new(),
            evt,
            downloads: root.join("downloads"),
        },
        events,
    )
}

fn fixture_server(port: u16) -> NewServer {
    NewServer {
        name: "fixture".into(),
        host: "127.0.0.1".into(),
        port,
        tls: false,
        starttls: false,
        username: None,
        password: None,
        priority: 1,
        max_connections: 2,
        pipeline_depth: 4,
        fill: false,
    }
}

mod ensure_headers;
mod failures;
mod header_cancel;
mod headers;
mod lifecycle;
mod media;
mod par2;
mod ram;
mod ram_parallel;
mod scheduler;
mod uu_downloads;

mod manage_downloads;

mod telemetry;

mod article_cache;

mod ram_visibility;

mod ram_names;
mod unsubscribe;

mod ram_uuencode;
