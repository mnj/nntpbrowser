use super::*;
use std::io::{Read, Write};

fn compressed(body: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    encoder.include_checksum(true).unwrap();
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

fn fixture_cache(worker: &Worker, id: &str, bytes: &[u8]) -> PathBuf {
    let legacy = worker.article_cache_path(1, id);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(legacy.with_extension("message-id"), id).unwrap();
    let path = legacy.with_extension("body.zst");
    std::fs::write(&path, bytes).unwrap();
    path
}

#[tokio::test]
async fn new_body_cache_is_small_checksummed_zstd_and_preserves_message_identity() {
    let (worker, _events) = fixture_worker();
    let body = b"Subject text and readable article body\r\n".repeat(20_000);
    let legacy = worker.article_cache_path(1, "one@test");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"old body").unwrap();
    worker.store_article_body(1, "one@test", &body).unwrap();
    let path = legacy.with_extension("body.zst");
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() < body.len() / 10);
    assert_eq!(zstd::stream::decode_all(&bytes[..]).unwrap(), body);
    assert_eq!(
        bytes[4] & 0x04,
        0x04,
        "zstd frame includes content checksum"
    );
    assert!(!legacy.exists());
    assert!(!path.with_extension("zst.pending").exists());
    assert_eq!(
        worker
            .cached_article_body(1, "one@test", usize::MAX)
            .unwrap(),
        body
    );
    assert!(worker
        .cached_article_body(1, "one@test", body.len() - 1)
        .is_none());
    std::fs::write(legacy.with_extension("message-id"), "other@test").unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
}

#[tokio::test]
async fn legacy_raw_body_lazily_migrates_without_changing_the_body_or_identity() {
    let (worker, _events) = fixture_worker();
    let legacy = worker.article_cache_path(1, "legacy@test");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(legacy.with_extension("message-id"), "legacy@test").unwrap();
    std::fs::write(&legacy, b"old article\r\n").unwrap();
    assert_eq!(
        worker.cached_article_body(1, "legacy@test", 100).unwrap(),
        b"old article\r\n"
    );
    assert!(worker.cached_article_body(1, "legacy@test", 2).is_none());
    assert!(legacy.with_extension("body.zst").is_file());
    assert!(!legacy.exists());
    assert_eq!(
        std::fs::read_to_string(legacy.with_extension("message-id")).unwrap(),
        "legacy@test"
    );
}

#[tokio::test]
async fn corrupt_truncated_and_trailing_invalid_frames_are_cache_misses() {
    let (worker, _events) = fixture_worker();
    let original = compressed(&b"readable body\r\n".repeat(50));
    let path = fixture_cache(&worker, "one@test", &original);
    for length in [0, 1, 5, original.len() / 2, original.len() - 1] {
        std::fs::write(&path, &original[..length]).unwrap();
        assert!(
            worker
                .cached_article_body(1, "one@test", usize::MAX)
                .is_none(),
            "length {length}"
        );
    }
    let mut corrupt = original.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, corrupt).unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
    let mut trailing = original.clone();
    trailing.extend_from_slice(&original[..original.len() - 1]);
    std::fs::write(&path, trailing).unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
    let mut trailing = original;
    trailing.extend_from_slice(b"garbage");
    std::fs::write(path, trailing).unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
}

#[tokio::test]
async fn cache_enforces_output_input_and_decoder_window_limits() {
    let (worker, _events) = fixture_worker();
    let oversized = vec![b'a'; 4 * 1024 * 1024 + 1];
    assert!(worker
        .store_article_body(1, "one@test", &oversized)
        .is_err());
    let path = fixture_cache(&worker, "one@test", &compressed(&oversized));
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
    std::fs::write(&path, vec![0; 4 * 1024 * 1024 + 64 * 1024 + 1]).unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
    // Valid empty frame declaring an 8 MiB window; the cache permits only 4 MiB.
    let frame = [0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x68, 0x01, 0x00, 0x00];
    let mut reference = zstd::stream::read::Decoder::new(&frame[..]).unwrap();
    reference.window_log_max(23).unwrap();
    let mut body = Vec::new();
    reference.read_to_end(&mut body).unwrap();
    assert!(body.is_empty());
    std::fs::write(path, frame).unwrap();
    assert!(worker
        .cached_article_body(1, "one@test", usize::MAX)
        .is_none());
}

#[tokio::test]
async fn failed_lazy_migration_preserves_a_valid_legacy_cache_hit() {
    let (worker, _events) = fixture_worker();
    let legacy = worker.article_cache_path(1, "legacy@test");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(legacy.with_extension("message-id"), "legacy@test").unwrap();
    std::fs::write(&legacy, b"legacy bytes\r\n").unwrap();
    // A directory blocks creating the pending file without relying on OS permissions.
    std::fs::create_dir(legacy.with_extension("body.zst.pending")).unwrap();
    assert_eq!(
        worker.cached_article_body(1, "legacy@test", 100).unwrap(),
        b"legacy bytes\r\n"
    );
    assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy bytes\r\n");
    assert!(!legacy.with_extension("body.zst").exists());
}

#[tokio::test]
async fn legacy_selection_avoids_network_and_corrupt_compressed_cache_refetches() {
    let hits = std::sync::Arc::new(AtomicUsize::new(0));
    let count = hits.clone();
    let addr = serve(move |mut socket| {
        let count = count.clone();
        async move {
            write_all(&mut socket, b"201 ready\r\n").await;
            assert_eq!(read_line(&mut socket).await, "CAPABILITIES");
            write_all(&mut socket, b"101 caps\r\nOVER\r\n.\r\n").await;
            loop {
                let command = read_line(&mut socket).await;
                if command.is_empty() {
                    break;
                }
                assert_eq!(command, "BODY <article.txt-1@test>");
                count.fetch_add(1, Ordering::SeqCst);
                write_multiline(&mut socket, b"refetched article body\r\n").await;
            }
        }
    })
    .await;
    let (mut worker, mut events) = fixture_worker();
    let sid = worker
        .catalog
        .insert_server(&fixture_server(addr.port()))
        .unwrap();
    let pid = super::media::add_media_post(&mut worker, sid, "Article", &[("article.txt", 20, 1)]);
    let legacy = worker.article_cache_path(pid, "article.txt-1@test");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, b"legacy article body\r\n").unwrap();
    std::fs::write(legacy.with_extension("message-id"), "article.txt-1@test").unwrap();
    worker.read_article(pid).await;
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    let compressed = legacy.with_extension("body.zst");
    assert!(compressed.is_file());
    assert!(!legacy.exists());
    assert!(std::iter::from_fn(||events.try_recv().ok()).any(|event|matches!(event,Event::ArticleContent { text,binary:false,.. } if text=="legacy article body\r\n")));
    let mut corrupt = std::fs::read(&compressed).unwrap();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&compressed, corrupt).unwrap();
    worker.read_article(pid).await;
    worker.read_article(pid).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let mut truncated = std::fs::read(&compressed).unwrap();
    truncated.pop();
    std::fs::write(&compressed, truncated).unwrap();
    worker.read_article(pid).await;
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(
        worker
            .cached_article_body(pid, "article.txt-1@test", 100)
            .unwrap(),
        b"refetched article body\r\n"
    );
}
