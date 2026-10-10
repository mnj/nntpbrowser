pub fn launch() -> eframe::Result {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() == 1 && matches!(args[0].as_str(), "--version" | "-V") {
        println!("NNTPBrowser {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    match crate::parse_cli(&args) {
        Ok(Some(command)) => {
            if let Err(error) = crate::run_cli(command) {
                eprintln!("{error}");
                std::process::exit(1);
            }
            return Ok(());
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1040.0, 680.0])
            .with_title("NNTPBrowser"),
        ..Default::default()
    };
    eframe::run_native(
        "nntpbrowser",
        native_options,
        Box::new(|cc| Ok(Box::new(crate::Browser::new(cc)))),
    )
}
