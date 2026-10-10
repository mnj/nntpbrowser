use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn temp_root(label: &str) -> PathBuf {
    static NEXT_ROOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "nntpbrowser-osf-{}-{}-{}-{label}",
        std::process::id(),
        NEXT_ROOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

fn rec(article: u64, subject: &str) -> Record {
    Record {
        article,
        date_unix: 1_700_000_000 + article as i64,
        date_raw: if article.is_multiple_of(2) {
            Some("not a date".into())
        } else {
            None
        },
        bytes: article * 10,
        lines: article as u32,
        message_id: format!("<{article}@example>"),
        subject: subject.into(),
        from: "Poster".into(),
    }
}

#[test]
fn slug_keeps_plain_names_and_hashes_the_rest() {
    assert_eq!(group_slug("alt.binaries.foo"), "alt.binaries.foo");
    assert_eq!(group_slug("a_b-c+d.e"), "a_b-c+d.e");
    assert!(group_slug("has space").starts_with("g_"));
    assert_eq!(group_slug("has space").len(), 2 + 16);
    assert!(group_slug("..").starts_with("g_"));
    assert!(group_slug("a..b").starts_with("g_"));
    assert_ne!(group_slug("a..b"), "a..b");
}

#[test]
fn round_trip_watermark_and_modes() {
    let root = temp_root("round");
    let store = HeaderStore::open(&root).unwrap();
    let records = vec![rec(1, "one"), rec(2, "two"), rec(4, "four")];
    store.append_range(7, "alt.test", &records).unwrap();
    assert_eq!(store.watermark(7, "alt.test").unwrap(), 4);
    let mut got = Vec::new();
    let bad = store
        .for_each(7, "alt.test", |record| got.push(record))
        .unwrap();
    assert!(bad.is_empty());
    assert_eq!(got, records);
    #[cfg(unix)]
    {
        let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }
    let file = fs::read_dir(root.join("7").join("alt.test"))
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            let name = entry.file_name();
            let name = name.to_str().unwrap();
            name.ends_with(".osf").then(|| entry.path())
        })
        .unwrap();
    #[cfg(unix)]
    {
        let file_mode = fs::metadata(file).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
    }
    #[cfg(not(unix))]
    let _ = file;
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn order_and_overlap_are_rejected() {
    let root = temp_root("order");
    let store = HeaderStore::open(&root).unwrap();
    let err = store
        .append_range(1, "alt.test", &[rec(2, "b"), rec(2, "b")])
        .unwrap_err();
    assert!(matches!(err, Error::Order));
    store
        .append_range(1, "alt.test", &[rec(1, "a"), rec(2, "b")])
        .unwrap();
    let err = store
        .append_range(1, "alt.test", &[rec(2, "again")])
        .unwrap_err();
    assert!(matches!(err, Error::Header(_)));
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn skipped_corrupt_block_is_not_reported() {
    let root = temp_root("bad");
    let store = HeaderStore::open(&root).unwrap();
    store
        .append_range_sized(1, "alt.test", &[rec(1, "one"), rec(2, "two")], 1)
        .unwrap();
    let dir = root.join("1").join("alt.test");
    let osf = fs::read_dir(&dir)
        .unwrap()
        .find_map(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_str()
                .unwrap()
                .ends_with(".osf")
                .then(|| entry.path())
        })
        .unwrap();
    let mut bytes = fs::read(&osf).unwrap();
    // First block payload starts after the 34-byte header and 16-byte block meta.
    let flip = HEADER_LEN + 16;
    bytes[flip] ^= 0xff;
    fs::write(&osf, &bytes).unwrap();

    let mut second = Vec::new();
    let bad = store
        .for_range(1, "alt.test", 2, 2, &mut |record| second.push(record))
        .unwrap();
    assert!(bad.is_empty(), "{bad:?}");
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].article, 2);

    let mut all = Vec::new();
    let bad = store
        .for_each(1, "alt.test", |record| all.push(record))
        .unwrap();
    assert_eq!(bad.len(), 1);
    assert_eq!(bad[0].first, 1);
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].article, 2);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn orphans_are_swept_and_many_files_compact() {
    let root = temp_root("compact");
    let store = HeaderStore::open(&root).unwrap();
    for n in 1..=17 {
        store.append_range(3, "alt.test", &[rec(n, "s")]).unwrap();
    }
    let dir = root.join("3").join("alt.test");
    fs::write(dir.join("r_999_999.osf"), b"orphan").unwrap();
    assert_eq!(store.sweep_orphans(3, "alt.test").unwrap(), 1);
    assert!(!dir.join("r_999_999.osf").exists());
    assert!(store.compact_if_needed(3, "alt.test").unwrap());
    let mut got = Vec::new();
    let bad = store
        .for_each(3, "alt.test", |record| got.push(record.article))
        .unwrap();
    assert!(bad.is_empty());
    assert_eq!(got, (1..=17).collect::<Vec<_>>());
    let manifest = fs::read_to_string(dir.join("manifest.json")).unwrap();
    assert_eq!(manifest.matches(".osf").count(), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn historical_ranges_read_in_order_and_compaction_preserves_holes() {
    let root = temp_root("historical");
    let store = HeaderStore::open(&root).unwrap();
    for n in 100..=116 {
        store.append_range(1, "alt.test", &[rec(n, "new")]).unwrap();
    }
    store
        .append_range(1, "alt.test", &[rec(1, "old"), rec(2, "old")])
        .unwrap();
    assert!(store.compact_if_needed(1, "alt.test").unwrap());
    assert_eq!(
        store.stored_ranges(1, "alt.test").unwrap(),
        vec![(1, 2), (100, 116)]
    );
    assert_eq!(store.watermark(1, "alt.test").unwrap(), 116);
    let mut articles = Vec::new();
    assert!(store
        .for_each(1, "alt.test", |record| articles.push(record.article))
        .unwrap()
        .is_empty());
    assert_eq!(articles, [vec![1, 2], (100..=116).collect()].concat());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn damaged_compaction_keeps_original_manifest_and_range_files() {
    let root = temp_root("damaged-compact");
    let store = HeaderStore::open(&root).unwrap();
    for n in 1..=17 {
        store
            .append_range(1, "alt.test", &[rec(n, "data")])
            .unwrap();
    }
    let dir = root.join("1/alt.test");
    let original = fs::read(dir.join("manifest.json")).unwrap();
    let path = dir.join("r_9_9.osf");
    let mut data = fs::read(&path).unwrap();
    data[HEADER_LEN + 16] ^= 0xff;
    fs::write(path, data).unwrap();
    assert!(store.compact_if_needed(1, "alt.test").is_err());
    assert_eq!(fs::read(dir.join("manifest.json")).unwrap(), original);
    assert_eq!(
        fs::read_dir(&dir)
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "osf"))
            .count(),
        17
    );
    let mut seen = Vec::new();
    let bad = store
        .for_each(1, "alt.test", |record| seen.push(record.article))
        .unwrap();
    assert_eq!(bad, vec![BadSpan { first: 9, last: 9 }]);
    assert_eq!(seen.len(), 16);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corrupt_index_count_is_rejected_before_allocating() {
    let root = temp_root("bad-index");
    let store = HeaderStore::open(&root).unwrap();
    store
        .append_range(1, "alt.test", &[rec(1, "data")])
        .unwrap();
    let path = root.join("1/alt.test/r_1_1.osf");
    let mut data = fs::read(&path).unwrap();
    data[22..26].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(path, data).unwrap();
    assert!(matches!(
        store.for_each(1, "alt.test", |_| {}),
        Err(Error::Header(_))
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn removed_server_cache_cannot_be_inherited_when_its_id_is_reused() {
    let root = temp_root("server-delete");
    let store = HeaderStore::open(&root).unwrap();
    store
        .append_range(1, "alt.test", &[rec(100, "old provider")])
        .unwrap();
    store
        .append_range(2, "alt.test", &[rec(200, "other provider")])
        .unwrap();
    store.remove_server(1).unwrap();
    store.remove_server(1).unwrap();
    assert_eq!(store.watermark(1, "alt.test").unwrap(), 0);
    assert!(store.stored_ranges(1, "alt.test").unwrap().is_empty());
    assert_eq!(store.watermark(2, "alt.test").unwrap(), 200);
    store
        .append_range(1, "alt.test", &[rec(1, "new provider")])
        .unwrap();
    let mut seen = Vec::new();
    assert!(store
        .for_each(1, "alt.test", |record| seen.push(record.subject))
        .unwrap()
        .is_empty());
    assert_eq!(seen, vec!["new provider"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn remove_group_deletes_header_journals_only_in_its_group_directory() {
    let root = temp_root("remove-group");
    let store = HeaderStore::open(&root).unwrap();
    store
        .append_range(1, "alt.remove", &[rec(10, "remove")])
        .unwrap();
    store
        .append_range(1, "alt.keep", &[rec(20, "keep")])
        .unwrap();
    store
        .append_range(2, "alt.remove", &[rec(30, "other-server")])
        .unwrap();
    fs::write(root.join("1/alt.remove/pending.journal"), b"unfinished").unwrap();
    store.remove_group(1, "alt.remove").unwrap();
    assert!(!root.join("1/alt.remove").exists());
    assert_eq!(store.watermark(1, "alt.keep").unwrap(), 20);
    assert_eq!(store.watermark(2, "alt.remove").unwrap(), 30);
    store.remove_group(1, "alt.remove").unwrap();
    assert_eq!(store.watermark(1, "alt.remove").unwrap(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn remove_group_refuses_symlinked_cache_ancestors_and_keeps_targets() {
    use std::os::unix::fs::symlink;
    for level in ["root", "server", "group"] {
        let root = temp_root(level);
        let outside = temp_root("outside-group");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"saved").unwrap();
        let store = HeaderStore::open(&root).unwrap();
        let link = match level {
            "root" => {
                fs::remove_dir(&root).unwrap();
                root.clone()
            }
            "server" => root.join("1"),
            _ => {
                fs::create_dir(root.join("1")).unwrap();
                root.join("1/alt.test")
            }
        };
        symlink(&outside, &link).unwrap();
        assert!(store.remove_group(1, "alt.test").is_err());
        assert_eq!(fs::read(outside.join("keep")).unwrap(), b"saved");
        fs::remove_file(link).unwrap();
        if root.exists() {
            fs::remove_dir_all(root).unwrap();
        }
        fs::remove_dir_all(outside).unwrap();
    }
}
