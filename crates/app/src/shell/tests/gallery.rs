use super::*;
use crate::shell::{gallery::GridGeometry, preferences::Preferences};

/// Inspect painted, clipped titles independently of the Gallery's request list.
/// The viewport fits two columns and two title rows; virtual overscan still
/// constructs a third row whose titles must not count as visible.
struct PaintedGallery {
    titles: HashSet<i64>,
    visible_cards: usize,
    first_row_columns: usize,
    row_height: f32,
    max_card_width: f32,
}

fn paint(
    ctx: &egui::Context,
    browser: &mut Browser,
    offset: f32,
    size: egui::Vec2,
) -> PaintedGallery {
    let indexes = (0..browser.posts.len()).collect::<Vec<_>>();
    let mut row_height = 0.0;
    let output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
            let id = ui.make_persistent_id(egui::IdSalt::new("gallery"));
            let mut scroll = egui::scroll_area::State::load(ctx, id).unwrap_or_default();
            scroll.offset.y = offset;
            scroll.store(ctx, id);
            row_height =
                GridGeometry::new(ui, browser.preferences_applied.gallery_tile_width).row_height;
            browser.gallery(ui, &indexes);
        },
    );
    let titles: Vec<(i64, f32)> = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::epaint::Shape::Text(text)
                if clipped.clip_rect.intersects(text.visual_bounding_rect()) =>
            {
                Some((
                    text.galley.text().strip_prefix("Image ")?.parse().ok()?,
                    text.pos.y,
                ))
            }
            _ => None,
        })
        .collect();
    let cards: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::epaint::Shape::Rect(rect)
                if rect.fill == theme::SURFACE && clipped.clip_rect.intersects(rect.rect) =>
            {
                assert!(
                    rect.rect.right() <= clipped.clip_rect.right() + 1.0,
                    "rightmost card extends outside the scroll viewport: {:?} vs {:?}",
                    rect.rect,
                    clipped.clip_rect
                );
                Some(rect.rect)
            }
            _ => None,
        })
        .collect();
    // Border and caption metrics can shift individual cards slightly; their
    // frame tops identify rows independently of the Gallery's grid calculation.
    let top = cards.iter().map(|rect| rect.top()).min_by(f32::total_cmp);
    let first_row_columns = cards
        .iter()
        .filter(|rect| top.is_some_and(|top| (rect.top() - top).abs() < 8.0))
        .count();
    let visible_cards = cards.len();
    let max_card_width = cards.iter().map(egui::Rect::width).fold(0.0, f32::max);
    output.drop_without_applying_deltas();
    PaintedGallery {
        titles: titles.into_iter().map(|(id, _)| id).collect(),
        visible_cards,
        first_row_columns,
        row_height,
        max_card_width,
    }
}

fn render(ctx: &egui::Context, browser: &mut Browser, offset: f32) -> HashSet<i64> {
    paint(ctx, browser, offset, egui::vec2(500.0, 500.0)).titles
}

#[test]
fn gallery_scroll_requests_visible_tiles_and_releases_old_pending_without_refetching_cache() {
    let mut browser = browser();
    browser.view = BrowseView::Gallery;
    browser.gallery_auto = true;
    browser.posts = (1..=120).map(|id| post(id, 1)).collect();
    browser.selected_group = Some(1);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let cached: Arc<[u8]> = Arc::from(&b"already loaded media bytes"[..]);
    browser.on_memory_preview(1, "cached.png".into(), cached.clone());

    let initial = render(&ctx, &mut browser, 0.0);
    assert_eq!(initial, HashSet::from([1, 2, 3, 4]));
    assert_eq!(browser.media_pending, HashSet::from([2, 3, 4]));
    assert_eq!(
        browser.media_viewport.as_ref().unwrap().1,
        vec![1, 2, 3, 4],
        "virtual overscan is rendered but must not start media fetches"
    );
    browser.media_progress.insert(2, (100, 1024));

    let row_height = paint(&ctx, &mut browser, 0.0, egui::vec2(500.0, 500.0)).row_height;
    let scrolled = render(&ctx, &mut browser, row_height * 20.0 + 4.0);
    assert_eq!(scrolled, HashSet::from([41, 42, 43, 44]));
    assert_eq!(browser.media_pending, scrolled);
    assert!(browser.media_progress.is_empty());
    assert!(Arc::ptr_eq(&browser.media_memory[&1].bytes, &cached));

    let returned = render(&ctx, &mut browser, 0.0);
    assert_eq!(returned, initial);
    assert_eq!(browser.media_pending, HashSet::from([2, 3, 4]));
    assert!(!browser.media_pending.contains(&1));
    assert!(Arc::ptr_eq(&browser.media_memory[&1].bytes, &cached));

    browser.gallery_auto = false;
    render(&ctx, &mut browser, 0.0);
    assert!(browser.media_pending.is_empty());
    assert!(browser.media_viewport.as_ref().unwrap().1.is_empty());
    assert!(Arc::ptr_eq(&browser.media_memory[&1].bytes, &cached));
}

#[test]
fn resizing_gallery_changes_columns_and_requests_only_clipped_visible_cards() {
    let mut browser = browser();
    browser.view = BrowseView::Gallery;
    browser.gallery_auto = true;
    browser.posts = (1..=240).map(|id| post(id, 1)).collect();
    browser.selected_group = Some(1);
    let ctx = egui::Context::default();
    theme::install(&ctx);
    let cached: Arc<[u8]> = Arc::from(b"cached".as_slice());
    browser.on_memory_preview(1, "cached.png".into(), cached.clone());
    let generation = browser.media_generation;
    browser.set_gallery_tile_width(140.0, false);
    let compact = paint(&ctx, &mut browser, 0.0, egui::vec2(900.0, 600.0));
    let compact_ids: HashSet<i64> = browser
        .media_viewport
        .as_ref()
        .unwrap()
        .1
        .iter()
        .copied()
        .collect();
    assert_eq!(compact.first_row_columns, 6);
    assert_eq!(compact.visible_cards, compact_ids.len());
    assert!(compact.titles.is_subset(&compact_ids));
    assert_eq!(
        browser.media_pending,
        compact_ids
            .difference(&HashSet::from([1]))
            .copied()
            .collect()
    );

    browser.set_gallery_tile_width(360.0, false);
    let large = paint(&ctx, &mut browser, 0.0, egui::vec2(900.0, 600.0));
    let large_ids: HashSet<i64> = browser
        .media_viewport
        .as_ref()
        .unwrap()
        .1
        .iter()
        .copied()
        .collect();
    assert_eq!(large.first_row_columns, 3);
    assert!(large.max_card_width <= 360.0);
    assert!(large.row_height > compact.row_height);
    assert!(large_ids.len() < compact_ids.len());
    assert_eq!(large.visible_cards, large_ids.len());
    assert!(large.titles.is_subset(&large_ids));
    assert_eq!(
        browser.media_pending,
        large_ids.difference(&HashSet::from([1])).copied().collect()
    );

    let scrolled = paint(
        &ctx,
        &mut browser,
        large.row_height * 20.0 + 4.0,
        egui::vec2(900.0, 600.0),
    );
    let scrolled_ids: HashSet<i64> = browser
        .media_viewport
        .as_ref()
        .unwrap()
        .1
        .iter()
        .copied()
        .collect();
    assert_eq!(scrolled.visible_cards, scrolled_ids.len());
    assert!(scrolled.titles.is_subset(&scrolled_ids));
    assert!(scrolled_ids.iter().all(|id| *id >= 61));
    assert!(scrolled_ids.is_disjoint(&large_ids));
    assert_eq!(browser.media_pending, scrolled_ids);
    assert_eq!(browser.media_generation, generation);
    assert!(Arc::ptr_eq(&browser.media_memory[&1].bytes, &cached));

    // A narrow viewport may fit one column, but its image must not stretch to
    // the full window width and push the card's captions below the viewport.
    let narrow = paint(&ctx, &mut browser, 0.0, egui::vec2(500.0, 600.0));
    assert_eq!(narrow.first_row_columns, 1);
    assert!(narrow.max_card_width <= 360.0);
    assert!(narrow.titles.contains(&1));
    assert_eq!(
        narrow.visible_cards,
        browser.media_viewport.as_ref().unwrap().1.len()
    );
}

#[test]
fn tile_size_persistence_preserves_ram_and_unapplied_settings() {
    let mut browser = browser();
    let cached: Arc<[u8]> = Arc::from(b"cached".as_slice());
    browser.on_memory_preview(1, "cached.png".into(), cached.clone());
    let generation = browser.media_generation;
    browser.preferences.nntp_timeout_secs = 99;
    let applied_timeout = browser.preferences_applied.nntp_timeout_secs;
    browser.set_gallery_tile_width(300.0, true);
    let (reloaded, error) = Preferences::load(browser.handle.data_dir());
    assert!(error.is_none());
    assert_eq!(reloaded.gallery_tile_width, 300.0);
    assert_eq!(reloaded.nntp_timeout_secs, applied_timeout);
    assert_eq!(browser.preferences.nntp_timeout_secs, 99);
    assert_eq!(browser.preferences.gallery_tile_width, 300.0);
    assert_eq!(browser.media_generation, generation);
    assert!(Arc::ptr_eq(&browser.media_memory[&1].bytes, &cached));
}

#[test]
fn gallery_requests_all_visible_uncached_tiles_without_a_four_preview_ceiling() {
    let mut browser = browser();
    browser.posts = (1..=60).map(|id| post(id, 1)).collect();
    browser.selected_group = Some(1);
    browser.on_memory_preview(1, "cached.png".into(), Arc::from(b"cached".as_slice()));
    let visible = (1..=50).collect::<Vec<_>>();
    browser.request_media(&visible);
    assert_eq!(browser.media_pending, (2..=50).collect::<HashSet<_>>());
    // Repainting does not duplicate requests or include offscreen tiles.
    browser.request_media(&visible);
    assert_eq!(browser.media_pending.len(), 49);
    assert!(!browser.media_pending.contains(&51));
    browser.request_media(&[50, 51]);
    assert_eq!(browser.media_pending, HashSet::from([50, 51]));
    assert!(browser.media_memory.contains_key(&1));
}
