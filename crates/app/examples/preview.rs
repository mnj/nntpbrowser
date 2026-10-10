//! Offline fixtures for reviewing native layouts without an NNTP account.
fn main() -> eframe::Result {
    // The fixture is for layout review, so use a deterministic software GL path.
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "true");
    if let Some(path) = std::env::args().nth(3) {
        std::env::set_var("EFRAME_SCREENSHOT_TO", path);
    }
    let view = std::env::args().nth(1).unwrap_or_else(|| "articles".into());
    let size = std::env::args().nth(2).unwrap_or_else(|| "1280x820".into());
    let (width, height) = if let Some((w, h)) = size.split_once('x') {
        (
            w.parse::<f32>().unwrap_or(1280.0),
            h.parse::<f32>().unwrap_or(820.0),
        )
    } else {
        (size.parse::<f32>().unwrap_or(1280.0), 820.0)
    };
    eframe::run_native(
        "NNTPBrowser — UI preview",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            viewport: eframe::egui::ViewportBuilder::default().with_inner_size([width, height]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::Browser::preview(cc, &view)))),
    )
}
