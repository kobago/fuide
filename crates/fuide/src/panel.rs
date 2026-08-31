//! Rectangular panel with a title chip that "cuts" the top edge (chamfer optional via `theme::Corners`).

use egui::{pos2, Align2, Color32, Id, Layout, Rect, Stroke, Ui, UiBuilder, Vec2};

use crate::{geom, theme};

/// Horizontal inset of the title chip from the panel's left edge.
pub const CHIP_INSET: f32 = 14.0;
pub const PAD_X: f32 = 12.0;
pub const PAD_Y: f32 = 18.0;

pub struct Panel {
    title: String,
    /// Optional second chip, drawn right-aligned (e.g. a live readout).
    tag: Option<(String, Color32)>,
    padding: Vec2,
}

impl Panel {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into().to_uppercase(),
            tag: None,
            padding: egui::vec2(PAD_X, PAD_Y),
        }
    }

    pub fn tag(mut self, text: impl Into<String>, color: Color32) -> Self {
        self.tag = Some((text.into().to_uppercase(), color));
        self
    }

    pub fn padding(mut self, x: f32, y: f32) -> Self {
        self.padding = egui::vec2(x, y);
        self
    }

    /// Draw the panel into an explicit rect and lay the contents out inside it (flow layout).
    pub fn show_rect<R>(
        self,
        ui: &mut Ui,
        rect: Rect,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let pal = theme::palette(ui.ctx());
        let c = theme::corners(ui.ctx()).panel;
        let ts = theme::type_scale(ui.ctx());
        let p = ui.painter();

        geom::fill(p, geom::hexagon_tl_br(rect, c), pal.bg_panel);
        geom::outline(
            p,
            geom::hexagon_tl_br(rect, c),
            Stroke::new(1.0, pal.accent_dim.gamma_multiply(0.8)),
        );

        // title chip straddling the top edge
        let inset = CHIP_INSET.max(c);
        let chip_h = (ts.label + 6.0).round();
        chip(
            p,
            pos2(rect.left() + inset, rect.top() - chip_h / 2.0),
            &self.title,
            pal.accent,
            &pal,
            ts.label,
        );
        if let Some((tag, color)) = &self.tag {
            let w = chip_width(p, tag, ts.label);
            chip(
                p,
                pos2(rect.right() - inset - w, rect.top() - chip_h / 2.0),
                tag,
                *color,
                &pal,
                ts.label,
            );
        }

        let inner = Rect::from_min_max(
            pos2(rect.left() + self.padding.x, rect.top() + self.padding.y),
            pos2(
                rect.right() - self.padding.x,
                rect.bottom() - self.padding.y,
            ),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(Id::new(("fuide-panel", &self.title)))
                .max_rect(inner)
                .layout(Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(inner.intersect(ui.clip_rect()));
        add_contents(&mut child)
    }
}

pub fn chip_width(p: &egui::Painter, text: &str, size: f32) -> f32 {
    p.layout_no_wrap(text.to_string(), theme::mono(size), Color32::WHITE)
        .size()
        .x
        + 16.0
}

/// Title chip (also used by dialogs).
pub fn chip(
    p: &egui::Painter,
    min: egui::Pos2,
    text: &str,
    color: Color32,
    pal: &theme::Palette,
    size: f32,
) {
    let rect = Rect::from_min_size(
        min,
        egui::vec2(chip_width(p, text, size), (size + 6.0).round()),
    );
    p.rect_filled(rect, egui::CornerRadius::ZERO, pal.bg_deep);
    p.rect_stroke(
        rect,
        egui::CornerRadius::ZERO,
        Stroke::new(1.0, pal.accent_dim),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::mono(size),
        color,
    );
}
