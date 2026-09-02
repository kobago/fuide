//! Rectangular panel with a title chip that "cuts" the top edge (chamfer optional via `theme::Corners`).

use egui::{
    pos2, Align2, Color32, Id, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, WidgetInfo,
    WidgetType,
};

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
        self.show_impl(ui, rect, None, add_contents).0
    }

    /// Like [`Panel::show_rect`], but the title chip is a click target that opens / closes the
    /// panel (drawn as `[-]` / `[+]` after the title). The caller owns the flag: it shrinks
    /// `rect` to a header strip while closed, and flips the flag when this returns `true`.
    pub fn show_collapsible_rect<R>(
        self,
        ui: &mut Ui,
        rect: Rect,
        open: bool,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> (R, bool) {
        self.show_impl(ui, rect, Some(open), add_contents)
    }

    fn show_impl<R>(
        self,
        ui: &mut Ui,
        rect: Rect,
        open: Option<bool>,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> (R, bool) {
        let pal = theme::palette(ui.ctx());
        let c = theme::corners(ui.ctx()).panel;
        let ts = theme::type_scale(ui.ctx());

        geom::fill(ui.painter(), geom::hexagon_tl_br(rect, c), pal.bg_panel);
        geom::outline(
            ui.painter(),
            geom::hexagon_tl_br(rect, c),
            Stroke::new(1.0, pal.accent_dim.gamma_multiply(0.8)),
        );

        // title chip straddling the top edge
        let inset = CHIP_INSET.max(c);
        let chip_h = (ts.label + 6.0).round();
        let title = match open {
            Some(true) => format!("{} [-]", self.title),
            Some(false) => format!("{} [+]", self.title),
            None => self.title.clone(),
        };
        let chip_min = pos2(rect.left() + inset, rect.top() - chip_h / 2.0);
        let mut toggled = false;
        let mut hovered = false;
        if let Some(is_open) = open {
            let chip_rect = Rect::from_min_size(
                chip_min,
                egui::vec2(chip_width(ui.painter(), &title, ts.label), chip_h),
            );
            let resp = ui.interact(
                chip_rect,
                Id::new(("fuide-panel-open", &self.title)),
                Sense::click(),
            );
            crate::agent::describe(&resp, || {
                WidgetInfo::selected(WidgetType::Checkbox, true, is_open, self.title.clone())
            });
            toggled = resp.clicked();
            hovered = resp.hovered();
        }
        let p = ui.painter();
        chip(
            p,
            chip_min,
            &title,
            if hovered { pal.text } else { pal.accent },
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
        (add_contents(&mut child), toggled)
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
