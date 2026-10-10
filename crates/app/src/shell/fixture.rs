//! Isolated offline states for native visual QA. Never reads user data.
use super::*;
mod groups;
mod tooltips;

impl Browser {
    pub fn preview(cc: &eframe::CreationContext<'_>, view: &str) -> Self {
        theme::install(&cc.egui_ctx);
        let dir =
            std::env::temp_dir().join(format!("nntpbrowser-ui-preview-{}", std::process::id()));
        let mut browser = Self::with_handle(Handle::start(dir));
        browser.preview_fixture = true;
        browser.process_stats = process_stats::ProcessStats::fixture();
        browser.servers = vec![
            preview_server(1, "Primary server", true),
            preview_server(2, "Backup server", false),
        ];
        browser.groups = [
            "alt.binaries.pictures.nature",
            "alt.binaries.photography",
            "alt.binaries.multimedia",
        ]
        .iter()
        .enumerate()
        .map(|(i, name)| GroupRow {
            id: i as i64 + 1,
            server_id: 1,
            name: (*name).into(),
            low: 1,
            high: 250_000,
            watermark: 250_000,
            subscribed: true,
            last_refresh: None,
        })
        .collect();
        browser.groups.push(GroupRow {
            id: 4,
            server_id: 2,
            name: "alt.binaries.pictures.nature".into(),
            low: 1,
            high: 100_000,
            watermark: 0,
            subscribed: true,
            last_refresh: None,
        });
        browser.posts = (0..120)
            .map(|i| {
                let title = [
                    "Northern coast — morning light.jpg",
                    "Mountain trails — summer collection.zip",
                    "Forest canopy after the rain.jpg",
                    "City architecture — detail study.jpg",
                    "Coastal panorama at sunset.jpg",
                    "Night sky — long exposure.jpg",
                ][i % 6];
                PostRow {
                    id: i as i64 + 1,
                    group_id: 1,
                    group_key: title.into(),
                    title: title.into(),
                    poster: "photo@example.net".into(),
                    posted_at: 1_791_586_000 - i as i64 * 1800,
                    total_bytes: 2_750_000 + i as i64 * 17_000,
                    file_count: if i % 6 == 1 { 4 } else { 1 },
                    segment_count: if i % 6 == 1 { 32 } else { 1 },
                    incomplete: i % 9 == 4,
                    has_image: i % 6 != 1,
                    has_video: false,
                    has_par2: i % 6 == 1,
                }
            })
            .collect();
        browser.selected_group = Some(1);
        browser.selected_server = Some(1);
        browser.open_post = Some(1);
        browser.selection.extend([1, 3]);
        browser.files = vec![FileRow {
            id: 1,
            post_id: 1,
            name: "Northern coast — morning light.jpg".into(),
            size: 2_750_000,
            expected_segments: 1,
            kind: engine::FileKind::Image,
        }];
        for i in 0..24 {
            let pixels: Vec<Color32> = (0..160)
                .flat_map(|y| {
                    (0..256).map(move |x| {
                        let ridge = 95 + ((x as f32 / 38.0 + i as f32).sin() * 22.0) as i32;
                        if y > ridge {
                            Color32::from_rgb(30 + (i * 7 % 30) as u8, 62 + (y / 5) as u8, 66)
                        } else {
                            Color32::from_rgb(
                                (45 + y / 3) as u8,
                                (88 + y / 4) as u8,
                                (122 + x / 8) as u8,
                            )
                        }
                    })
                })
                .collect();
            let key = format!("post:{}", i + 1);
            let texture = cc.egui_ctx.load_texture(
                &key,
                egui::ColorImage::new([256, 160], pixels),
                egui::TextureOptions::LINEAR,
            );
            browser.insert_texture(key, texture);
        }
        browser.notice = "Ready · 120 articles cached locally".into();
        match view {
            "sidebar-collapsed" => browser.sidebar_open = false,
            "article-text" => {
                browser.posts[0].title = "Welcome to the photography group".into();
                browser.posts[0].has_image = false;
                browser.posts[0].total_bytes = 1450;
                browser.article_requested = Some(1);
                browser.article_content = Some(ArticleContent {
                    binary: false,
                    text: "Hello everyone,\n\nThis week's theme is light and shadow. Share your favourite photos, discuss the techniques you used, and ask questions about your next shoot.\n\nWhen you select an article, its body is fetched automatically and cached locally for later reading. Attachments appear in the file list.\n\nHappy shooting!\nThe photography group".into(),
                });
                browser.files.clear();
            }
            "headers" => {
                browser.header_progress = Some(HeaderProgress {
                    received: 5000,
                    stored: 4000,
                    first: 10001,
                    last: 20000,
                    completed: 6000,
                    total: 10000,
                    started: Instant::now() - Duration::from_secs(12),
                });
                browser.posts_loading = true;
            }
            "empty" => {
                browser.posts.clear();
                browser.feed_dirty = true;
                browser.selection.clear();
                browser.open_post = None;
                browser.files.clear();
                browser.notice = "Ready · no cached articles".into();
            }
            "gallery" | "gallery-small" | "gallery-large" => {
                browser.view = BrowseView::Gallery;
                let tile_width = match view {
                    "gallery-small" => 140.0,
                    "gallery-large" => 360.0,
                    _ => 220.0,
                };
                browser.set_gallery_tile_width(tile_width, false);
                browser.filter = FeedFilter::Media;
            }
            "gallery-memory" => {
                browser.view = BrowseView::Gallery;
                browser.filter = FeedFilter::Media;
                browser.preview_post = Some(1);
                let name = "Northern coast — morning light.png".to_string();
                browser.posts[0].title = name.clone();
                browser.files[0].name = name.clone();
                browser.on_memory_preview(
                    1,
                    name,
                    Arc::from(include_bytes!("fixture/coast.png").as_slice()),
                );
                if let Some(texture) = browser.textures.get("post:1").cloned() {
                    browser.insert_texture(browser.media_memory[&1].key(1, true), texture);
                }
            }
            "errors" => {
                browser.record_error(
                    "overview: server reply timed out while fetching articles 249001–250000",
                    "Article headers · Primary server / alt.binaries.pictures.nature",
                );
                browser.record_error(
                    "Article missing on every enabled server (430).",
                    "Download #7 · Northern coast — morning light.jpg",
                );
                browser.record_error(
                    "connection failed: authentication failed",
                    "Server · Backup server",
                );
                browser.notice = "connection failed: authentication failed".into();
                browser.errors_open = true;
            }
            "options" | "options-account" | "options-downloads" | "options-appearance"
            | "options-network" | "options-storage" => {
                browser.options_open = true;
                browser.editing = Some(1);
                browser.form = form_from(&browser.servers[0]);
                let page = match view {
                    "options-account" => options::SettingsPage::Account,
                    "options-downloads" => options::SettingsPage::Downloads,
                    _ => options::SettingsPage::Connection,
                };
                cc.egui_ctx
                    .data_mut(|data| data.insert_temp(egui::Id::new("settings-page"), page));
                let section = match view {
                    "options-appearance" => options::SettingsSection::Appearance,
                    "options-network" => options::SettingsSection::Network,
                    "options-storage" => options::SettingsSection::Storage,
                    _ => options::SettingsSection::Servers,
                };
                cc.egui_ctx
                    .data_mut(|data| data.insert_temp(egui::Id::new("settings-section"), section));
            }
            "group-layouts-real"
            | "group-layouts-real-indent"
            | "group-layouts-real-all"
            | "group-layouts-real-empty" => {
                browser.picker_server = Some(1);
                browser.samples.insert(1, groups::sample(true));
                browser.picker_filter = match view {
                    "group-layouts-real" | "group-layouts-real-indent" => "alt.binaries.pictures",
                    "group-layouts-real-empty" => "does.not.exist",
                    _ => "",
                }
                .into();
                browser.open_group_layouts(&cc.egui_ctx);
                if view == "group-layouts-real-indent" {
                    group_layouts::preview_style(&cc.egui_ctx, "indent");
                }
            }
            view if view == "group-layouts" || view.starts_with("group-layouts-") => {
                browser.group_layouts_open = true;
                let key = view.strip_prefix("group-layouts-").unwrap_or("muted");
                let style = key.split("-articles").next().unwrap_or("muted");
                group_layouts::preview_style(&cc.egui_ctx, style);
                if key.contains("-articles") {
                    group_layouts::preview_sort(&cc.egui_ctx, true, key.ends_with("-desc"));
                }
            }
            "groups" | "groups-loading" | "groups-sorted" => {
                browser.groups_open = true;
                browser.picker_server = Some(1);
                browser
                    .samples
                    .insert(1, groups::sample(view != "groups-loading"));
                browser.picker_loading = view == "groups-loading";
                if view == "groups-sorted" {
                    browser
                        .picker_sort
                        .preview_column(subscriptions::sort::Column::Articles);
                }
                browser.picker_dirty = true;
            }
            "queue"
            | "recovery"
            | "traffic"
            | "connections"
            | "downloads-completed"
            | "downloads-delete" => {
                browser.screen = Screen::Queue;
                let states = if view == "recovery" {
                    ["recovery_headers", "recovery_fetching", "repair", "paused"]
                } else {
                    ["fetching", "paused", "done", "failed"]
                };
                browser.jobs = states
                    .iter()
                    .enumerate()
                    .map(|(i, state)| JobRow {
                        id: i as i64 + 1,
                        post_id: Some(i as i64 + 1),
                        name: browser.posts[i].title.clone(),
                        dest_dir: browser
                            .handle
                            .data_dir()
                            .join(format!(
                                "downloads/{}-{}",
                                [
                                    "coastal-photo",
                                    "mountain-trails",
                                    "forest-canopy",
                                    "city-detail"
                                ][i],
                                i + 1
                            ))
                            .display()
                            .to_string(),
                        dest_path: browser
                            .handle
                            .data_dir()
                            .join(format!("downloads/photo{}.jpg", i + 1))
                            .display()
                            .to_string(),
                        state: (*state).into(),
                        bytes_done: if *state == "done" {
                            8_000_000
                        } else {
                            3_000_000
                        },
                        bytes_total: 8_000_000,
                        error: if *state == "failed" {
                            Some("Article missing on every enabled server (430).".into())
                        } else {
                            None
                        },
                        password: None,
                    })
                    .collect();
                let now = Instant::now();
                let mut rate = TransferRate::new(0, now - Duration::from_secs(5));
                rate.observe(12_500_000, now);
                browser.speeds.insert(1, rate);
                browser.network_stats.insert((1, 1), (2, 4, 2_500_000.0));
                let done = browser.jobs[2].clone();
                let items = [
                    "Forest canopy after the rain.jpg",
                    "Field notes.txt",
                    "Coastal panorama.jpg",
                ]
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    let path = PathBuf::from(&done.dest_dir).join(name);
                    if index != 1 {
                        if let Some(texture) = browser
                            .textures
                            .get(&format!("post:{}", index + 1))
                            .cloned()
                        {
                            browser.insert_texture(format!("file:{}", path.display()), texture);
                        }
                    }
                    LibItem {
                        job_id: done.id,
                        path,
                        title: done.name.clone(),
                        kind: if index == 1 {
                            LibKind::File
                        } else {
                            LibKind::Image
                        },
                        size: if index == 1 { 1500 } else { 2_750_000 },
                    }
                })
                .collect::<Vec<_>>();
                browser.library.images = Arc::new(vec![0, 2]);
                browser.library.items = Arc::new(items);
                browser.notice = "Ready · downloads and saved files".into();
                browser.record_error(
                    "Article missing on every enabled server (430).",
                    "Download #4 · City architecture — detail study.jpg",
                );
                if matches!(view, "downloads-completed" | "downloads-delete") {
                    queue::preview_completed(
                        &cc.egui_ctx,
                        if view == "downloads-delete" {
                            Some((done, 3))
                        } else {
                            None
                        },
                    );
                }
                tooltips::seed(&mut browser, view);
            }
            _ => {}
        }
        browser
    }
}

fn preview_server(id: i64, name: &str, fill: bool) -> ServerRow {
    ServerRow {
        id,
        name: name.into(),
        host: "news.example.net".into(),
        port: 563,
        tls: true,
        starttls: false,
        username: Some("reader".into()),
        password: None,
        priority: id as i32,
        max_connections: 8,
        pipeline_depth: 8,
        fill,
        enabled: true,
        header_compression: "xzver".into(),
    }
}
