#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod git;

fn main() -> eframe::Result {
    // `fuide-git --mcp`: stdio MCP bridge to the running app (see `fuide::agent::bridge`)
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        std::process::exit(fuide::agent::bridge::run("git", "FUIDE Git"));
    }
    fuide::devshot::install_trace_logger();
    let start = std::env::args().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FUIDE Git")
            .with_app_id("fuide-git")
            .with_decorations(false)
            .with_transparent(true)
            .with_has_shadow(false)
            .with_inner_size([1380.0, 860.0])
            .with_min_inner_size([1100.0, 680.0]),
        ..Default::default()
    };
    eframe::run_native(
        "FUIDE Git",
        options,
        Box::new(move |cc| Ok(Box::new(app::GitApp::new(cc, start)))),
    )
}
