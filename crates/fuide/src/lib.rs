//! fuide — FUI Develop Environment: Futuristic UI (FUI) building blocks for egui 0.36.
//!
//! Grammar: dark translucent ground + one glowing accent + thin lines + constant subtle motion.
//! - [`theme`]: palettes (CYAN / AMBER / GREEN), fonts (Orbitron display, Share Tech Mono data), style install
//! - [`shell`]: frameless window shell (square body, glow, title/status bars, resize)
//! - [`panel`]: panel with a title chip
//! - [`table`]: sortable column list (header hit areas, hover, selection, tag cells)
//! - [`widgets`]: nav tab, button, segment bar, arc gauge, lamp, log feed, readout
//! - [`fx`]: scanlines, scan band
//! - [`devshot`]: `FUIDE_SCREENSHOT` self-capture hook for visual checks
//! - [`dialog`]: modal dialog (dimmed backdrop, chip title, glowing outline, button row)
//! - [`settings`]: per-app theme settings (palette / corners / density) — a small config file
//!   plus a settings window opened as a child viewport (`Cmd+,`)
//! - [`fontmetrics`]: sfnt line metrics; baseline correction for fallback (CJK) fonts
//! - [`fmt`]: telemetry formatting (`T+HH:MM:SS.s` uptime)
//! - [`geom`]: polygons and glow strokes
//!
//! Corners are square by default. 45° chamfers are an opt-in flourish: `theme::set_corners(&ctx, Corners::CHAMFER)`.

pub mod devshot;
pub mod dialog;
pub mod fmt;
pub mod fontmetrics;
pub mod fx;
pub mod geom;
pub mod panel;
pub mod settings;
pub mod shell;
pub mod table;
pub mod theme;
pub mod widgets;

pub use dialog::Dialog;
pub use panel::Panel;
pub use settings::{Settings, SettingsWindow};
pub use shell::Shell;
pub use theme::{
    corners, display, display_galley, display_text, mono, palette, type_scale, Corners, Palette,
    PaletteKind, TypeScale,
};
