use super::*;

fn finished_job(id: i64, path: &Path) -> JobRow {
    JobRow {
        id,
        post_id: None,
        name: format!("Download {id}"),
        dest_dir: path.display().to_string(),
        dest_path: path.display().to_string(),
        state: "done".into(),
        bytes_done: 10,
        bytes_total: 10,
        error: None,
        password: None,
    }
}

fn wait_library(browser: &mut Browser) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while browser.library.loading() {
        browser.pump_library();
        assert!(Instant::now() < deadline, "library worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn library_scans_finished_jobs_in_background_and_unchanged_jobs_reuse_the_cache() {
    let mut browser = browser();
    let dir = browser.handle.data_dir().join("download");
    std::fs::create_dir_all(dir.join("album")).unwrap();
    std::fs::write(dir.join("album/photo.jpg"), b"image fixture").unwrap();
    std::fs::write(dir.join("archive.zip"), b"packed fixture").unwrap();
    std::fs::write(dir.join("readme.nfo"), b"file fixture").unwrap();
    std::fs::create_dir_all(dir.join(".uu-parts-1")).unwrap();
    std::fs::write(dir.join(".uu-parts-1/1.body"), b"encoded staging fixture").unwrap();
    let job = finished_job(1, &dir);
    browser.on_event(Event::Jobs(vec![job.clone()]));
    assert!(browser.library.loading());
    wait_library(&mut browser);
    assert_eq!(browser.library.items.len(), 3);
    assert!(browser.library.items.iter().all(|item| item.job_id == 1));
    assert!(
        browser
            .library
            .items
            .iter()
            .any(|item| item.path.ends_with("readme.nfo")
                && item.size == b"file fixture".len() as u64)
    );
    assert!(browser
        .library
        .items
        .iter()
        .any(|item| item.kind == LibKind::File));
    assert_eq!(browser.library.images.len(), 1);
    let original = browser.library.items.clone();
    std::fs::write(dir.join("extra.png"), b"new fixture").unwrap();
    browser.on_event(Event::Jobs(vec![job]));
    browser.pump_library();
    assert!(Arc::ptr_eq(&original, &browser.library.items));
    assert!(!browser.library.loading());
    browser.library.request(&browser.jobs, true);
    wait_library(&mut browser);
    assert_eq!(browser.library.items.len(), 4);
}

#[test]
fn pending_library_scan_uses_latest_finished_jobs_and_preserves_overlay_file() {
    let mut browser = browser();
    let root = browser.handle.data_dir();
    let earlier = root.join("a-download");
    let later = root.join("b-download");
    std::fs::create_dir_all(&earlier).unwrap();
    std::fs::create_dir_all(&later).unwrap();
    std::fs::write(earlier.join("a.jpg"), b"image fixture").unwrap();
    std::fs::write(later.join("b.jpg"), b"image fixture").unwrap();
    let first = finished_job(1, &later);
    let second = finished_job(2, &earlier);
    browser.on_event(Event::Jobs(vec![first.clone()]));
    wait_library(&mut browser);
    browser.overlay = Some(0);
    // Two changes before polling must discard the first scan's older snapshot.
    browser.library.request(std::slice::from_ref(&first), true);
    browser.on_event(Event::Jobs(vec![first, second]));
    wait_library(&mut browser);
    assert_eq!(browser.library.items.len(), 2);
    assert_eq!(browser.overlay, Some(1));
    let picked = browser
        .library
        .items
        .iter()
        .filter(|item| item.kind == LibKind::Image)
        .nth(browser.overlay.unwrap())
        .unwrap();
    assert_eq!(picked.path, later.join("b.jpg"));
    assert_eq!(picked.job_id, 1);
    assert!(browser
        .library
        .items
        .iter()
        .any(|item| item.path == earlier.join("a.jpg") && item.job_id == 2));
    // Removing a completed download must remove its indexed files and stale overlay.
    browser.on_event(Event::Jobs(vec![finished_job(2, &earlier)]));
    wait_library(&mut browser);
    assert_eq!(browser.library.items.len(), 1);
    assert_eq!(browser.library.items[0].job_id, 2);
    assert_eq!(browser.overlay, None);
}

#[test]
fn lightbox_cache_retains_two_recent_images_without_evicting_grid_thumbnails() {
    let mut browser = browser();
    let context = egui::Context::default();
    for key in ["file:grid", "full:first", "full:second", "full:third"] {
        let texture = context.load_texture(
            key,
            egui::ColorImage::filled([1, 1], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        browser.insert_texture(key.into(), texture);
    }
    assert!(!browser.textures.contains_key("full:first"));
    assert!(browser.textures.contains_key("full:second"));
    assert!(browser.textures.contains_key("full:third"));
    assert!(browser.textures.contains_key("file:grid"));
}
