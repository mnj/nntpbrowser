use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn temp_db() -> PathBuf {
    static NEXT_DB: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "nntpbrowser-cat-{}-{}-{}.sqlite",
        std::process::id(),
        NEXT_DB.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn server(name: &str) -> NewServer {
    NewServer {
        name: name.into(),
        host: "news.example".into(),
        port: 563,
        tls: true,
        starttls: false,
        username: Some("reader".into()),
        password: Some("s3cret-pass".into()),
        priority: 1,
        max_connections: 4,
        pipeline_depth: 8,
        fill: false,
    }
}

fn incoming(key: &str, title: &str, posted_at: i64, files: Vec<IncomingFile>) -> IncomingPost {
    IncomingPost {
        group_key: key.into(),
        title: title.into(),
        poster: "Foo".into(),
        posted_at,
        files,
    }
}

fn file(name: &str, size: i64, expected: i64, kind: FileKind) -> IncomingFile {
    IncomingFile {
        name: name.into(),
        size,
        expected_segments: expected,
        kind,
    }
}

mod queue;

mod bulk_queue;
mod jobs;
mod posts;
mod repair;
mod servers;
mod unsubscribe;
