//! Palette + egui style installation.
//!
//! Values come from the FUI design tokens (CYAN / AMBER / GREEN). One app, one palette.
//! Widgets never hard-code colors: they read the palette back via [`palette`].

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Align2, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Galley,
    Id, Painter, Pos2, Rect, Stroke, Theme, Visuals,
};

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub accent: Color32,
    pub accent_dim: Color32,
    pub warn: Color32,
    pub danger: Color32,
    pub ok: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub bg_deep: Color32,
    pub bg_panel: Color32,
}

impl Palette {
    /// Tactical — military / spaceship console. The default.
    pub fn cyan() -> Self {
        Self {
            accent: Color32::from_rgb(0x00, 0xE5, 0xFF),
            accent_dim: Color32::from_rgb(0x00, 0x78, 0x8C),
            warn: Color32::from_rgb(0xFF, 0xAA, 0x28),
            danger: Color32::from_rgb(0xFF, 0x46, 0x5A),
            ok: Color32::from_rgb(0x50, 0xFF, 0xA0),
            text: Color32::from_rgb(0xAA, 0xE6, 0xF0),
            text_dim: Color32::from_rgb(0x5A, 0x8C, 0x9B),
            bg_deep: Color32::from_rgba_unmultiplied(6, 12, 18, 235),
            bg_panel: Color32::from_rgba_unmultiplied(10, 22, 32, 199),
        }
    }

    /// Industrial — heavy machinery / reactor.
    pub fn amber() -> Self {
        Self {
            accent: Color32::from_rgb(0xFF, 0xB0, 0x20),
            accent_dim: Color32::from_rgb(0x8C, 0x60, 0x10),
            warn: Color32::from_rgb(0x00, 0xE5, 0xFF),
            danger: Color32::from_rgb(0xFF, 0x46, 0x5A),
            ok: Color32::from_rgb(0x50, 0xFF, 0xA0),
            text: Color32::from_rgb(0xF0, 0xDC, 0xB4),
            text_dim: Color32::from_rgb(0x96, 0x82, 0x5A),
            bg_deep: Color32::from_rgba_unmultiplied(18, 12, 4, 235),
            bg_panel: Color32::from_rgba_unmultiplied(30, 22, 8, 199),
        }
    }

    /// Phosphor terminal — green CRT.
    pub fn green() -> Self {
        Self {
            accent: Color32::from_rgb(0x46, 0xFF, 0x8C),
            accent_dim: Color32::from_rgb(0x1E, 0x82, 0x46),
            warn: Color32::from_rgb(0xFF, 0xAA, 0x28),
            danger: Color32::from_rgb(0xFF, 0x46, 0x5A),
            ok: Color32::from_rgb(0x00, 0xE5, 0xFF),
            text: Color32::from_rgb(0xB4, 0xF0, 0xC8),
            text_dim: Color32::from_rgb(0x5A, 0x96, 0x6E),
            bg_deep: Color32::from_rgba_unmultiplied(4, 16, 8, 235),
            bg_panel: Color32::from_rgba_unmultiplied(8, 28, 14, 199),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::cyan()
    }
}

const PALETTE_ID: &str = "fuide::palette";
const CORNERS_ID: &str = "fuide::corners";
const TYPE_ID: &str = "fuide::type";

/// Font sizes (logical px) used by every part of the kit. Defaults are sized to read like a
/// native macOS list at 1x (Finder body ≈ 13px): Share Tech Mono is condensed, so `data` is a
/// touch larger than that, and secondary text stays close to `data` rather than a step below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TypeScale {
    /// Window title (display face).
    pub title: f32,
    /// Section headings, tab names, column headers (display face).
    pub heading: f32,
    /// Primary data: list rows, readout values, inputs (mono).
    pub data: f32,
    /// Chips, status bar, log lines, secondary labels (mono).
    pub label: f32,
    /// Footnotes and key hints (mono).
    pub small: f32,
    /// Row height for lists / log feeds.
    pub row: f32,
    /// Extra letter spacing for the display face, in em. Orbitron is tight and reads badly
    /// without tracking, so headings/tabs/breadcrumbs are always drawn through [`display_text`].
    pub tracking: f32,
}

impl TypeScale {
    /// Finder-comparable sizes at 1x.
    pub const NORMAL: Self = Self {
        title: 20.0,
        heading: 13.0,
        data: 13.5,
        label: 13.0,
        small: 12.0,
        row: 24.0,
        tracking: 0.12,
    };

    /// The original dense instrument-panel sizes.
    pub const COMPACT: Self = Self {
        title: 17.0,
        heading: 11.0,
        data: 12.0,
        label: 11.0,
        small: 9.5,
        row: 20.0,
        tracking: 0.08,
    };

    pub fn scaled(self, f: f32) -> Self {
        Self {
            title: self.title * f,
            heading: self.heading * f,
            data: self.data * f,
            label: self.label * f,
            small: self.small * f,
            row: (self.row * f).round(),
            tracking: self.tracking,
        }
    }
}

impl Default for TypeScale {
    fn default() -> Self {
        Self::NORMAL
    }
}

/// Read the type scale back (widgets use this).
pub fn type_scale(ctx: &Context) -> TypeScale {
    ctx.data(|d| d.get_temp::<TypeScale>(Id::new(TYPE_ID)))
        .unwrap_or_default()
}

/// Change the type scale at runtime. Default is [`TypeScale::NORMAL`].
pub fn set_type_scale(ctx: &Context, scale: TypeScale) {
    ctx.data_mut(|d| d.insert_temp(Id::new(TYPE_ID), scale));
    ctx.all_styles_mut(|style| style.override_font_id = Some(mono(scale.data)));
}

/// Corner treatment of the shell / panels / tabs / buttons, in px of 45° cut.
/// **Default is square (all zero).** Chamfers are an optional, lower-priority flourish —
/// enable them only when explicitly asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corners {
    pub window: f32,
    pub panel: f32,
    pub tab: f32,
    pub button: f32,
}

impl Corners {
    /// Square corners everywhere (default).
    pub const SQUARE: Self = Self {
        window: 0.0,
        panel: 0.0,
        tab: 0.0,
        button: 0.0,
    };

    /// Optional military-hardware look: 26 / 14 / 12 / 7 px cuts.
    pub const CHAMFER: Self = Self {
        window: 26.0,
        panel: 14.0,
        tab: 12.0,
        button: 7.0,
    };
}

impl Default for Corners {
    fn default() -> Self {
        Self::SQUARE
    }
}

/// Read the corner settings back (widgets use this).
pub fn corners(ctx: &Context) -> Corners {
    ctx.data(|d| d.get_temp::<Corners>(Id::new(CORNERS_ID)))
        .unwrap_or_default()
}

/// Change the corner treatment at runtime. Default is [`Corners::SQUARE`].
pub fn set_corners(ctx: &Context, corners: Corners) {
    ctx.data_mut(|d| d.insert_temp(Id::new(CORNERS_ID), corners));
}

/// Display face (Orbitron). Headings, tab names, large numbers. Latin upper-case only.
pub const DISPLAY: &str = "fuide-display";

/// Font for headings / tabs / big numbers.
pub fn display(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(DISPLAY.into()))
}

/// Font for data / logs / labels.
pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

/// Lay out display-face text with the palette's letter spacing (`TypeScale::tracking`).
/// Use this (not `painter.text` + `display(..)`) for every heading / tab / title.
pub fn display_galley(
    p: &Painter,
    text: impl Into<String>,
    size: f32,
    color: Color32,
) -> std::sync::Arc<Galley> {
    let tracking = type_scale(p.ctx()).tracking;
    let format = TextFormat {
        font_id: display(size),
        color,
        extra_letter_spacing: size * tracking,
        ..Default::default()
    };
    p.layout_job(LayoutJob::single_section(text.into(), format))
}

/// Paint display-face text with tracking, anchored like `painter.text`. Returns the painted rect.
pub fn display_text(
    p: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl Into<String>,
    size: f32,
    color: Color32,
) -> Rect {
    let galley = display_galley(p, text, size, color);
    let rect = anchor.anchor_size(pos, galley.size());
    p.galley(rect.min, galley, color);
    rect
}

/// Read the installed palette back (widgets use this so they never hard-code colors).
pub fn palette(ctx: &Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palette>(Id::new(PALETTE_ID)))
        .unwrap_or_default()
}

/// Extra font to register as a *fallback* (appended to the end of the families),
/// e.g. a CJK system font so non-Latin file names don't render as tofu.
/// Raw bytes are kept so the face can be registered once per primary face with the
/// baseline correction that face needs (see [`crate::fontmetrics`]).
pub struct FallbackFont {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Face index inside a `.ttc`; 0 for `.ttf` / `.otf`.
    pub index: u32,
}

/// macOS: pick up a Hiragino face so Japanese text renders. Empty on other systems / if missing.
pub fn macos_cjk_fallback() -> Vec<FallbackFont> {
    let candidates = [
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
    ];
    for c in candidates {
        if let Ok(bytes) = std::fs::read(c) {
            return vec![FallbackFont {
                name: "system-cjk".into(),
                bytes,
                index: 0,
            }];
        }
    }
    Vec::new()
}

/// Install fonts + palette + widget styling. Call once at start-up.
pub fn install(ctx: &Context, palette: Palette, fallbacks: Vec<FallbackFont>) {
    install_fonts(ctx, fallbacks);
    apply_palette(ctx, palette);
}

/// Swap the palette at runtime (fonts untouched).
pub fn apply_palette(ctx: &Context, palette: Palette) {
    let p = palette.clone();
    ctx.set_theme(Theme::Dark);
    ctx.all_styles_mut(|style| {
        let v = &mut style.visuals;
        *v = Visuals::dark();
        v.override_text_color = Some(p.text);
        v.window_fill = Color32::TRANSPARENT;
        v.panel_fill = Color32::TRANSPARENT;
        v.extreme_bg_color = p.bg_deep;
        v.faint_bg_color = p.accent.gamma_multiply(0.04);
        v.selection.bg_fill = p.accent.gamma_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.window_stroke = Stroke::new(1.0, p.accent_dim);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.accent_dim.gamma_multiply(0.5));
        v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
        v.widgets.noninteractive.corner_radius = CornerRadius::ZERO;
        for (w, glow) in [
            (&mut v.widgets.inactive, 0.25f32),
            (&mut v.widgets.hovered, 0.6),
            (&mut v.widgets.active, 1.0),
            (&mut v.widgets.open, 0.6),
        ] {
            w.bg_fill = p.accent_dim.gamma_multiply(0.28);
            w.weak_bg_fill = p.accent_dim.gamma_multiply(0.18);
            w.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(glow));
            w.fg_stroke = Stroke::new(1.2, p.accent.gamma_multiply(0.4 + 0.6 * glow));
            w.corner_radius = CornerRadius::ZERO;
            w.expansion = 0.0;
        }
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = false;
        // Default text should be the mono face (data readouts).
        style.override_font_id = Some(mono(TypeScale::default().data));
    });
    ctx.data_mut(|d| d.insert_temp(Id::new(PALETTE_ID), palette));
}

const ORBITRON: &[u8] = include_bytes!("../../../assets/fonts/Orbitron.ttf");
const SHARE_TECH: &[u8] = include_bytes!("../../../assets/fonts/ShareTechMono.ttf");

fn install_fonts(ctx: &Context, fallbacks: Vec<FallbackFont>) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("orbitron".into(), Arc::new(FontData::from_static(ORBITRON)));
    fonts.font_data.insert(
        "sharetech".into(),
        Arc::new(FontData::from_static(SHARE_TECH)),
    );
    fonts
        .families
        .entry(FontFamily::Name(DISPLAY.into()))
        .or_default()
        .insert(0, "orbitron".into());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "sharetech".into());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "sharetech".into());
    // Each fallback is registered twice — once per primary face — with the vertical tweak that
    // lands its glyphs on that primary's baseline. The bytes are leaked once and shared.
    let primaries: [(&str, &[u8], &[FontFamily]); 2] = [
        (
            "mono",
            SHARE_TECH,
            &[FontFamily::Monospace, FontFamily::Proportional],
        ),
        ("display", ORBITRON, &[FontFamily::Name(DISPLAY.into())]),
    ];
    for fb in fallbacks {
        let face_metrics = crate::fontmetrics::line_metrics(&fb.bytes, fb.index);
        let bytes: &'static [u8] = Box::leak(fb.bytes.into_boxed_slice());
        for (tag, primary, families) in primaries.iter() {
            let y_offset_factor = match (face_metrics, crate::fontmetrics::line_metrics(primary, 0))
            {
                (Some(face), Some(prim)) => {
                    crate::fontmetrics::fallback_y_offset_factor(prim, face)
                }
                _ => 0.0,
            };
            let key = format!("{}-{tag}", fb.name);
            let mut data = FontData::from_static(bytes);
            data.index = fb.index;
            data.tweak.y_offset_factor = y_offset_factor;
            fonts.font_data.insert(key.clone(), Arc::new(data));
            for fam in families.iter() {
                fonts
                    .families
                    .entry(fam.clone())
                    .or_default()
                    .push(key.clone());
            }
        }
    }
    ctx.set_fonts(fonts);
}
