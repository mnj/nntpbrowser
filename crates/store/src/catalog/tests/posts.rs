use super::*;

#[test]
fn upsert_replaces_files_and_pages_by_keyset() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let posts = vec![
        incoming("b", "Beta", 20, vec![file("b.jpg", 10, 1, FileKind::Image)]),
        incoming(
            "a",
            "Alpha",
            30,
            vec![
                file("a.par2", 5, 2, FileKind::Par2),
                file("a.rar", 8, 0, FileKind::Archive),
            ],
        ),
        incoming("c", "Gamma", 10, vec![file("c.nfo", 1, 1, FileKind::Nfo)]),
    ];
    catalog.upsert_posts(gid, &posts).unwrap();
    assert_eq!(catalog.post_count(gid).unwrap(), 3);
    let page = catalog.page_posts(gid, None, 2, None).unwrap();
    assert_eq!(page.len(), 2);
    assert_eq!(page[0].title, "Alpha");
    assert!(page[0].has_par2);
    assert!(page[0].incomplete);
    assert_eq!(page[0].segment_count, 2);
    assert_eq!(page[0].total_bytes, 13);
    assert_eq!(page[1].title, "Beta");
    assert!(page[1].has_image);
    let next = catalog
        .page_posts(gid, Some((page[1].posted_at, page[1].id)), 2, None)
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].title, "Gamma");

    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "a",
                "Alpha two",
                30,
                vec![file("a.par2", 9, 3, FileKind::Par2)],
            )],
        )
        .unwrap();
    let alpha = catalog.page_posts(gid, None, 1, Some("Alpha")).unwrap();
    assert_eq!(alpha[0].title, "Alpha two");
    assert!(!alpha[0].incomplete);
    assert_eq!(alpha[0].segment_count, 3);
    let files = catalog.files_for_post(alpha[0].id).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "a.par2");
    assert_eq!(files[0].size, 9);
    let _ = fs::remove_file(&path);
}

#[test]
fn merge_posts_accumulates_parts() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .merge_posts(
            gid,
            &[incoming(
                "name\nfoo",
                "Name",
                10,
                vec![file("file.bin", 4, 2, FileKind::Data)],
            )],
        )
        .unwrap();
    catalog
        .merge_posts(
            gid,
            &[incoming(
                "name\nfoo",
                "Name",
                11,
                vec![file("file.bin", 6, 2, FileKind::Data)],
            )],
        )
        .unwrap();
    let posts = catalog.page_posts(gid, None, 10, None).unwrap();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].title, "Name");
    assert_eq!(posts[0].posted_at, 11);
    assert_eq!(posts[0].total_bytes, 10);
    assert_eq!(posts[0].segment_count, 2);
    let files = catalog.files_for_post(posts[0].id).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].size, 10);
    let _ = fs::remove_file(&path);
}

#[test]
fn indexed_lookup_and_equal_date_pagination_survive_reopen() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[
                incoming("a", "Alpha_%", 10, vec![]),
                incoming("b", "Beta", 10, vec![]),
                incoming("c", "Gamma", 10, vec![]),
            ],
        )
        .unwrap();
    let page = catalog.page_posts(gid, None, 2, None).unwrap();
    let cursor = page.last().map(|post| (post.posted_at, post.id));
    let remaining = catalog.page_posts(gid, cursor, 2, None).unwrap();
    assert_eq!(
        page.iter()
            .map(|post| post.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Gamma", "Beta"]
    );
    assert_eq!(remaining[0].title, "Alpha_%");
    assert_eq!(
        catalog.page_posts(gid, None, 20, Some("_%")).unwrap().len(),
        1
    );
    assert_eq!(
        catalog.post_by_id(page[0].id).unwrap(),
        Some(page[0].clone())
    );
    assert!(catalog.post_by_id(i64::MAX).unwrap().is_none());
    assert_eq!(catalog.group_by_id(gid).unwrap().unwrap().name, "alt.test");
    assert_eq!(catalog.server_by_id(sid).unwrap().unwrap().name, "main");
    drop(catalog);
    let reopened = Catalog::open(&path).unwrap();
    assert_eq!(reopened.page_posts(gid, None, 20, None).unwrap().len(), 3);
    drop(reopened);
    let _ = fs::remove_file(path);
}

#[test]
fn large_article_replacement_does_not_hit_sqlite_parameter_limit() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    let files = (0..33_000)
        .map(|index| file(&format!("file-{index}.bin"), 1, 1, FileKind::Data))
        .collect();
    catalog
        .upsert_posts(gid, &[incoming("large", "Large", 1, files)])
        .unwrap();
    let post = catalog.page_posts(gid, None, 1, None).unwrap().remove(0);
    assert_eq!(post.file_count, 33_000);
    assert_eq!(post.total_bytes, 33_000);
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "large",
                "Large",
                1,
                vec![file("file-32999.bin", 5, 1, FileKind::Data)],
            )],
        )
        .unwrap();
    let remaining = catalog.files_for_post(post.id).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "file-32999.bin");
    assert_eq!(remaining[0].size, 5);
    drop(catalog);
    let _ = fs::remove_file(path);
}

#[test]
fn video_predicate_matches_native_and_legacy_files_in_pages_and_lookup() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[
                incoming(
                    "typed",
                    "Typed video",
                    4,
                    vec![file("clip.webm", 100, 3, FileKind::Video)],
                ),
                incoming(
                    "legacy",
                    "Legacy video",
                    3,
                    vec![file("clip.MP4", 100, 3, FileKind::Data)],
                ),
                incoming(
                    "image",
                    "Image",
                    2,
                    vec![file("photo.jpg", 100, 3, FileKind::Image)],
                ),
                incoming(
                    "data",
                    "Data",
                    1,
                    vec![file("notes.mp4.txt", 100, 1, FileKind::Data)],
                ),
            ],
        )
        .unwrap();
    let posts = catalog.page_posts(gid, None, 10, None).unwrap();
    assert_eq!(
        posts.iter().map(|post| post.has_video).collect::<Vec<_>>(),
        vec![true, true, false, false]
    );
    assert!(posts[2].has_image);
    for post in &posts {
        assert_eq!(catalog.post_by_id(post.id).unwrap().as_ref(), Some(post));
    }
    assert_eq!(
        catalog.files_for_post(posts[0].id).unwrap()[0].kind,
        FileKind::Video
    );
    assert_eq!(
        catalog.files_for_post(posts[1].id).unwrap()[0].kind,
        FileKind::Video
    );
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "typed",
                "Now data",
                4,
                vec![file("file.bin", 100, 1, FileKind::Data)],
            )],
        )
        .unwrap();
    assert!(!catalog.post_by_id(posts[0].id).unwrap().unwrap().has_video);
    drop(catalog);
    let _ = fs::remove_file(path);
}

#[test]
fn date_filter_searches_all_cached_articles_before_keyset_paging() {
    let path = temp_db();
    let catalog = Catalog::open(&path).unwrap();
    let sid = catalog.insert_server(&server("dates")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test.dates").unwrap();
    let posts = (0..220)
        .map(|i| {
            incoming(
                &i.to_string(),
                if i % 2 == 0 { "Photo" } else { "Other" },
                i,
                vec![],
            )
        })
        .collect::<Vec<_>>();
    catalog.upsert_posts(gid, &posts).unwrap();
    assert_eq!(
        catalog.page_posts(gid, None, 80, None).unwrap()[0].posted_at,
        219
    );
    let first = catalog
        .page_posts_date_range(gid, None, 2, Some("Photo"), Some(20), Some(30))
        .unwrap();
    assert_eq!(
        first.iter().map(|post| post.posted_at).collect::<Vec<_>>(),
        vec![30, 28]
    );
    let cursor = first.last().map(|post| (post.posted_at, post.id));
    let rest = catalog
        .page_posts_date_range(gid, cursor, 80, Some("Photo"), Some(20), Some(30))
        .unwrap();
    assert_eq!(
        rest.iter().map(|post| post.posted_at).collect::<Vec<_>>(),
        vec![26, 24, 22, 20]
    );
    let upper = catalog
        .page_posts_date_range(gid, None, 80, None, None, Some(2))
        .unwrap();
    assert_eq!(
        upper.iter().map(|post| post.posted_at).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert!(catalog
        .page_posts(gid, None, 300, None)
        .unwrap()
        .iter()
        .any(|post| post.posted_at == 0));
    assert!(catalog
        .page_posts_date_range(gid, None, 80, None, Some(40), Some(20))
        .unwrap()
        .is_empty());
    drop(catalog);
    let _ = fs::remove_file(path);
}

#[test]
fn corrected_spaced_names_keep_post_identity_and_merge_legacy_file_sizes_once() {
    for journal in [false, true] {
        let catalog = Catalog::open(temp_db()).unwrap();
        let sid = catalog.insert_server(&server("main")).unwrap();
        let gid = catalog.subscribe(sid, "alt.test").unwrap();
        let subject = r#"Holiday - "My holiday photo.jpg" yEnc (1/2)"#;
        let legacy = crate::parse_subject_legacy(subject, "Foo");
        catalog
            .upsert_posts(
                gid,
                &[incoming(
                    &legacy.group_key,
                    &legacy.title,
                    1,
                    vec![file("photo.jpg", 3, 2, FileKind::Image)],
                )],
            )
            .unwrap();
        let old_id = catalog.page_posts(gid, None, 80, None).unwrap()[0].id;
        let corrected =
            crate::parse_subject(r#"Holiday - "My holiday photo.jpg" yEnc (2/2)"#, "Foo");
        assert_eq!(corrected.group_key, legacy.group_key);
        let post = incoming(
            &corrected.group_key,
            &corrected.title,
            2,
            vec![file("My holiday photo.jpg", 3, 2, FileKind::Image)],
        );
        if journal {
            let snapshots = catalog.merged_post_snapshots(gid, &[post]).unwrap();
            catalog.upsert_posts(gid, &snapshots).unwrap();
            catalog.upsert_posts(gid, &snapshots).unwrap(); // crash replay stays absolute
        } else {
            catalog.merge_posts(gid, &[post]).unwrap();
        }
        let posts = catalog.page_posts(gid, None, 80, None).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, old_id);
        assert_eq!(posts[0].title, "Holiday");
        assert_eq!(posts[0].total_bytes, 6);
        let files = catalog.files_for_post(old_id).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "My holiday photo.jpg");
        assert_eq!(files[0].size, 6);
    }
}

#[test]
fn spaced_name_alias_correction_does_not_merge_a_distinct_short_named_file() {
    let catalog = Catalog::open(temp_db()).unwrap();
    let sid = catalog.insert_server(&server("main")).unwrap();
    let gid = catalog.subscribe(sid, "alt.test").unwrap();
    catalog
        .upsert_posts(
            gid,
            &[incoming(
                "holiday\nfoo",
                "Holiday",
                1,
                vec![file("photo.jpg", 3, 1, FileKind::Image)],
            )],
        )
        .unwrap();
    let addition = incoming(
        "holiday\nfoo",
        "Holiday",
        2,
        vec![file("My holiday photo.jpg", 4, 1, FileKind::Image)],
    );
    let snapshots = catalog.merged_post_snapshots(gid, &[addition]).unwrap();
    catalog.upsert_posts(gid, &snapshots).unwrap();
    let post = &catalog.page_posts(gid, None, 80, None).unwrap()[0];
    assert_eq!(catalog.files_for_post(post.id).unwrap().len(), 2);
    assert_eq!(post.total_bytes, 7);
}
