//! Dev aid shared by all tools: `FUIDE_SCREENSHOT=/path/out.tga cargo run` captures the window after
//! a short warm-up (`FUIDE_SCREENSHOT_FRAME`, default 45) and exits. Convert with `sips -s format png`.
//! `FUIDE_DEV_FRAMELOG=1` prints `frame N t=…` to stderr every 30 frames (repaint-rate checks). Written as uncompressed TGA (no extra deps); convert with `sips -s format png`.
//! `FUIDE_DEV_EMBED=1` embeds child viewports (the settings window) in the main window as
//! `egui::Window`s so the screenshot includes them — eframe cannot screenshot a child viewport
//! (immediate ones drop the request; deferred ones stall the event loop for ~1 s and then stop
//! repainting on macOS, seen with eframe 0.36.1). `FUIDE_DEV_TRACE=1` prints eframe / egui `log`
//! output (repaint scheduling, viewport lifecycle).

use std::path::PathBuf;

/// Dev aid: `FUIDE_DEV_TRACE=1` prints eframe / egui-winit `log` output (trace level: repaint
/// scheduling, viewport events) to stderr. Call once before `eframe::run_native`.
pub fn install_trace_logger() {
    if std::env::var_os("FUIDE_DEV_TRACE").is_none() {
        return;
    }
    struct L(std::time::Instant);
    impl log::Log for L {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            m.target().starts_with("eframe") || m.target().starts_with("egui")
        }
        fn log(&self, r: &log::Record<'_>) {
            if self.enabled(r.metadata()) {
                eprintln!(
                    "{:8.3} {:5} {} {}",
                    self.0.elapsed().as_secs_f64(),
                    r.level(),
                    r.target(),
                    r.args()
                );
            }
        }
        fn flush(&self) {}
    }
    let _ = log::set_boxed_logger(Box::new(L(std::time::Instant::now())));
    log::set_max_level(log::LevelFilter::Trace);
}

pub struct DevShot {
    path: Option<PathBuf>,
    frames: u32,
    at: u32,
    requested: bool,
    framelog: bool,
    /// `FUIDE_DEV_RESIZE_AT=<frame>`: send `InnerSize` (+400 px wide) at that frame and log when the
    /// new size reaches `ui()` — separates event/repaint latency from presentation latency.
    resize_at: Option<u32>,
    resize_sent: Option<(f64, f32)>,
}

impl DevShot {
    pub fn from_env() -> Self {
        Self {
            path: std::env::var_os("FUIDE_SCREENSHOT").map(PathBuf::from),
            frames: 0,
            at: std::env::var("FUIDE_SCREENSHOT_FRAME")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(45),
            requested: false,
            framelog: std::env::var_os("FUIDE_DEV_FRAMELOG").is_some(),
            resize_at: std::env::var("FUIDE_DEV_RESIZE_AT")
                .ok()
                .and_then(|v| v.parse().ok()),
            resize_sent: None,
        }
    }

    /// Call once per frame from `App::ui`.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.frames == 0 && std::env::var_os("FUIDE_DEV_EMBED").is_some() {
            ctx.set_embed_viewports(true);
        }
        self.frames += 1;
        if self.framelog && self.frames.is_multiple_of(30) {
            eprintln!("frame {} t={:.2}", self.frames, ctx.input(|i| i.time));
        }
        let (t, size) = ctx.input(|i| {
            (
                i.time,
                i.viewport().inner_rect.map(|r| r.width()).unwrap_or(0.0),
            )
        });
        if self.resize_at == Some(self.frames) {
            let target = size + 400.0;
            let h = ctx
                .input(|i| i.viewport().inner_rect.map(|r| r.height()))
                .unwrap_or(800.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(target, h)));
            self.resize_sent = Some((t, size));
            eprintln!(
                "resize: requested {size:.0} -> {target:.0} at frame {} t={t:.4}",
                self.frames
            );
        } else if let Some((t0, w0)) = self.resize_sent {
            if (size - w0).abs() > 1.0 {
                eprintln!(
                    "resize: ui() sees {size:.0} at frame {} t={t:.4} (+{:.1} ms)",
                    self.frames,
                    (t - t0) * 1000.0
                );
                self.resize_sent = None;
            } else {
                eprintln!(
                    "resize: frame {} still {size:.0} (+{:.1} ms)",
                    self.frames,
                    (t - t0) * 1000.0
                );
            }
        }
        let Some(path) = self.path.clone() else {
            return;
        };
        if self.frames == self.at && !self.requested {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            match write_tga(&path, &img) {
                Ok(()) => eprintln!("screenshot written: {}", path.display()),
                Err(e) => eprintln!("screenshot failed: {e}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Write a `ColorImage` as an uncompressed 32-bit TGA (no extra deps).
pub fn write_tga(path: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let mut buf = Vec::with_capacity(18 + w * h * 4);
    buf.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    buf.extend_from_slice(&(w as u16).to_le_bytes());
    buf.extend_from_slice(&(h as u16).to_le_bytes());
    buf.push(32); // bpp
    buf.push(0x28); // 8 alpha bits, top-left origin
    for px in &img.pixels {
        buf.extend_from_slice(&[px.b(), px.g(), px.r(), px.a()]);
    }
    std::fs::write(path, buf)
}
