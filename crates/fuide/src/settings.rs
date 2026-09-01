//! Per-app theme settings: what the user can change at runtime (palette, corners, density),
//! persisted to a small `key=value` file, and the settings window that edits them.
//!
//! The window is a real OS window (an egui child viewport) drawn with its own [`Shell`] so it
//! matches the main window. Fonts, palette and style live on the shared [`egui::Context`], so a
//! change made in the settings window shows up in the main window on the same frame.
//!
//! ```ignore
//! // start-up
//! let settings = Settings::load(APP_ID).unwrap_or_else(|| Settings::new(PaletteKind::Cyan));
//! theme::install(&cc.egui_ctx, settings.palette.palette(), fallbacks);
//! settings.apply(&cc.egui_ctx);
//! // every frame, after the main shell
//! if self.settings_win.show(ui.ctx(), &mut self.settings, "My Tool") {
//!     self.settings.save(APP_ID).ok();
//! }
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::{
    pos2, vec2, Align2, Context, Key, Rect, Sense, Stroke, Ui, ViewportBuilder, ViewportId,
};

use crate::theme::{self, Corners, PaletteKind, TypeScale};
use crate::{geom, widgets, Panel, Shell};

/// Everything the settings window edits. Add fields here, in `Default`, `to_string`/`parse`
/// and `apply`; the window draws one section per field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub palette: PaletteKind,
    /// 45° chamfers on window / panels / tabs / buttons.
    pub chamfer: bool,
    /// Denser type scale ([`TypeScale::COMPACT`] instead of [`TypeScale::NORMAL`]).
    pub compact: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(PaletteKind::Cyan)
    }
}

impl Settings {
    /// Defaults with the app's own default palette.
    pub fn new(palette: PaletteKind) -> Self {
        Self {
            palette,
            chamfer: false,
            compact: false,
        }
    }

    /// Push the settings into the context (palette, corners, type scale).
    pub fn apply(&self, ctx: &Context) {
        theme::apply_palette(ctx, self.palette.palette());
        theme::set_corners(
            ctx,
            if self.chamfer {
                Corners::CHAMFER
            } else {
                Corners::SQUARE
            },
        );
        theme::set_type_scale(
            ctx,
            if self.compact {
                TypeScale::COMPACT
            } else {
                TypeScale::NORMAL
            },
        );
    }

    // ---------------------------------------------------------------- persistence

    /// Where `app_id`'s settings live. `FUIDE_CONFIG_DIR` overrides the directory (dev aid:
    /// screenshots and tests must not touch the real file). Otherwise
    /// `~/Library/Application Support/FUIDE/<app_id>.conf` on macOS,
    /// `$XDG_CONFIG_HOME/fuide/<app_id>.conf` (or `~/.config/fuide/…`) elsewhere.
    pub fn path(app_id: &str) -> Option<PathBuf> {
        let dir = if let Some(d) = std::env::var_os("FUIDE_CONFIG_DIR") {
            PathBuf::from(d)
        } else if cfg!(target_os = "macos") {
            PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/FUIDE")
        } else if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            PathBuf::from(x).join("fuide")
        } else {
            PathBuf::from(std::env::var_os("HOME")?).join(".config/fuide")
        };
        Some(dir.join(format!("{app_id}.conf")))
    }

    /// Read the saved settings; `None` if there is no file (or it cannot be read).
    /// Unknown keys are ignored and missing keys keep their defaults, so old files stay valid.
    pub fn load(app_id: &str) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(app_id)?).ok()?;
        Some(Self::parse(&text))
    }

    /// Write the settings (creates the directory).
    pub fn save(&self, app_id: &str) -> std::io::Result<()> {
        let path = Self::path(app_id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no config directory (HOME unset)",
            )
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_string())
    }

    pub fn parse(text: &str) -> Self {
        let mut s = Self::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "palette" => {
                    if let Some(p) = PaletteKind::from_name(v) {
                        s.palette = p;
                    }
                }
                "chamfer" => s.chamfer = v == "true",
                "compact" => s.compact = v == "true",
                _ => {}
            }
        }
        s
    }
}

impl std::fmt::Display for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "# FUIDE settings")?;
        writeln!(f, "palette={}", self.palette.name())?;
        writeln!(f, "chamfer={}", self.chamfer)?;
        writeln!(f, "compact={}", self.compact)
    }
}

// -------------------------------------------------------------------------------- window

pub const WINDOW_SIZE: [f32; 2] = [400.0, 460.0];

/// State shared with the viewport callback (deferred viewports run on their own repaint schedule,
/// so their closure must be `Send + Sync + 'static`).
struct Shared {
    settings: Settings,
    /// The callback changed `settings` since the parent last looked.
    settings_edited: bool,
    close: bool,
}

/// The settings window's open/closed state. Keep one in the app; call [`SettingsWindow::show`]
/// every frame.
pub struct SettingsWindow {
    open: bool,
    shared: Arc<Mutex<Shared>>,
}

impl Default for SettingsWindow {
    fn default() -> Self {
        Self {
            open: false,
            shared: Arc::new(Mutex::new(Shared {
                settings: Settings::default(),
                settings_edited: false,
                close: false,
            })),
        }
    }
}

impl SettingsWindow {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.shared.lock().unwrap().close = false;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open();
        }
    }

    /// Draw the window while open. It is a *deferred* child viewport: its own native window,
    /// repainted on its own schedule (and, being a tool window, only on input). The immediate
    /// kind would repaint both windows whenever either needs a frame, and eframe 0.36 cannot
    /// screenshot it. Dev aid: `FUIDE_DEV_EMBED=1` (see [`crate::devshot`]) draws it inside the
    /// main window instead so `FUIDE_SCREENSHOT` can capture it. `app_name` goes in the subtitle. Changes are applied to `ctx` the
    /// moment they are made; this returns `true` on the parent frame that first sees them (the
    /// caller saves / logs). Closes on the shell's close button, `Esc`, or `Cmd+W`.
    pub fn show(&mut self, ctx: &Context, settings: &mut Settings, app_name: &str) -> bool {
        if !self.open {
            return false;
        }
        // Sync with the callback's copy: pick up its edits, otherwise push ours (shortcuts).
        let mut changed = false;
        {
            let mut sh = self.shared.lock().unwrap();
            if sh.close {
                sh.close = false;
                self.open = false;
                return false;
            }
            if sh.settings != *settings {
                if sh.settings_edited {
                    *settings = sh.settings.clone();
                    changed = true;
                } else {
                    sh.settings = settings.clone();
                }
            }
            sh.settings_edited = false;
        }
        let shared = Arc::clone(&self.shared);
        let app_name = app_name.to_string();
        ctx.show_viewport_deferred(
            ViewportId::from_hash_of("fuide-settings"),
            ViewportBuilder::default()
                .with_title("Settings")
                .with_decorations(false)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_resizable(false)
                .with_inner_size(WINDOW_SIZE),
            move |ui, _class| {
                let mut sh = shared.lock().unwrap();
                let before = sh.settings.clone();
                Shell::new("Settings")
                    .subtitle(&app_name)
                    .tool_window()
                    .status_left("ESC CLOSE")
                    .show(ui, |ui| settings_body(ui, &mut sh.settings));
                if sh.settings != before {
                    sh.settings.apply(ui.ctx());
                    sh.settings_edited = true;
                }
                let close = ui.input(|i| {
                    i.viewport().close_requested()
                        || i.key_pressed(Key::Escape)
                        || (i.modifiers.command && i.key_pressed(Key::W))
                });
                if close {
                    sh.close = true;
                }
                if sh.settings_edited || close {
                    // the parent is asleep between its own repaints; wake it to save / close
                    ui.ctx().request_repaint_of(ui.ctx().parent_viewport_id());
                }
            },
        );
        changed
    }
}

fn settings_body(ui: &mut Ui, s: &mut Settings) {
    let c = ui.max_rect();
    let ts = theme::type_scale(ui.ctx());
    let top = c.top() + 10.0;
    let palette_h = 12.0 + 3.0 * (ts.row + 2.0 + 3.0) + 18.0 + 6.0;
    let palette_rect = Rect::from_min_size(pos2(c.left(), top), vec2(c.width(), palette_h));
    let look_rect = Rect::from_min_max(pos2(c.left(), palette_rect.bottom() + 22.0), c.max);

    Panel::new("Palette").show_rect(ui, palette_rect, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for kind in PaletteKind::ALL {
            let label = format!("{}  {}", kind.name(), kind.blurb());
            let resp = widgets::nav_tab(ui, &label, s.palette == kind);
            // swatch of the palette's accent at the right edge
            let r = resp.rect;
            let sw = Rect::from_center_size(pos2(r.right() - 16.0, r.center().y), vec2(14.0, 14.0));
            let accent = kind.palette().accent;
            let p = ui.painter();
            geom::fill(p, geom::rect(sw), accent.gamma_multiply(0.85));
            geom::outline(p, geom::rect(sw), Stroke::new(1.0, accent));
            if resp.clicked() {
                s.palette = kind;
            }
        }
    });

    Panel::new("Look").show_rect(ui, look_rect, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;
        widgets::section_label(ui, "Corners");
        ui.horizontal(|ui| {
            let mut square = !s.chamfer;
            let mut chamfer = s.chamfer;
            if widgets::toggle_chip(ui, "square", &mut square).clicked() {
                s.chamfer = false;
            }
            if widgets::toggle_chip(ui, "chamfer", &mut chamfer).clicked() {
                s.chamfer = true;
            }
        });
        ui.add_space(4.0);
        widgets::section_label(ui, "Density");
        ui.horizontal(|ui| {
            let mut normal = !s.compact;
            let mut compact = s.compact;
            if widgets::toggle_chip(ui, "normal", &mut normal).clicked() {
                s.compact = false;
            }
            if widgets::toggle_chip(ui, "compact", &mut compact).clicked() {
                s.compact = true;
            }
        });
        ui.add_space(8.0);
        let pal = theme::palette(ui.ctx());
        let (fr, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), ts.small + 6.0), Sense::hover());
        ui.painter().text(
            pos2(fr.left(), fr.center().y),
            Align2::LEFT_CENTER,
            "CHANGES APPLY AT ONCE AND ARE SAVED",
            theme::mono(ts.small),
            pal.text_dim,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = Settings {
            palette: PaletteKind::Amber,
            chamfer: true,
            compact: false,
        };
        assert_eq!(Settings::parse(&s.to_string()), s);
    }

    #[test]
    fn parse_is_lenient() {
        let s = Settings::parse("# comment\npalette = green\nbogus=1\nchamfer=yes\n\ncompact=true");
        assert_eq!(s.palette, PaletteKind::Green);
        assert!(!s.chamfer); // only the literal `true` switches it on
        assert!(s.compact);
        assert_eq!(Settings::parse("palette=nope"), Settings::default());
    }

    #[test]
    fn save_and_load_use_config_dir_override() {
        let dir = std::env::temp_dir().join(format!("fuide-settings-test-{}", std::process::id()));
        // SAFETY: tests in this module run single-threaded with respect to this variable
        // (no other test reads it), and the value is a valid path.
        unsafe { std::env::set_var("FUIDE_CONFIG_DIR", &dir) };
        let s = Settings::new(PaletteKind::Green);
        s.save("unit-test").unwrap();
        assert_eq!(Settings::load("unit-test"), Some(s));
        std::fs::remove_dir_all(&dir).unwrap();
        unsafe { std::env::remove_var("FUIDE_CONFIG_DIR") };
    }
}
