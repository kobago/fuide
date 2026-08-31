//! Modal dialog: dimmed backdrop + a centred panel with a title chip and a glowing outline.
//! Input below the dialog is blocked (built on `egui::Modal`); `Esc` / backdrop click asks to close.

use egui::{pos2, vec2, Color32, Context, Frame, Id, Layout, Modal, Rect, Shape, Ui};

/// Fade duration for opening and closing, seconds.
pub const FADE_SECS: f64 = 0.15;

use crate::{geom, panel, theme};

pub const PAD_X: f32 = 18.0;
pub const PAD_Y: f32 = 22.0;

pub struct Dialog {
    title: String,
    tag: Option<(String, Color32)>,
    width: f32,
    /// Outline colour; defaults to the accent. Use `danger` for destructive dialogs.
    outline: Option<Color32>,
    /// Draw the title chip on the top edge (off for cards whose body already says it, e.g. `ERROR`).
    show_title: bool,
}

pub struct DialogResponse<R> {
    /// Contents' return value. `None` once the dialog has faded out completely.
    pub inner: Option<R>,
    /// `Esc` was pressed or the backdrop was clicked (only while `open`).
    pub should_close: bool,
    /// The close fade has finished: drop the dialog state now.
    pub finished: bool,
}

/// Per-dialog animation clock, kept in `Context` data.
#[derive(Clone, Copy)]
struct Anim {
    opened_at: f64,
    closing_at: Option<f64>,
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

impl Dialog {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into().to_uppercase(),
            tag: None,
            width: 420.0,
            outline: None,
            show_title: true,
        }
    }

    pub fn tag(mut self, text: impl Into<String>, color: Color32) -> Self {
        self.tag = Some((text.into().to_uppercase(), color));
        self
    }

    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    pub fn outline(mut self, color: Color32) -> Self {
        self.outline = Some(color);
        self
    }

    /// Hide the title chip (the title is still used as the dialog's id).
    pub fn show_title(mut self, show: bool) -> Self {
        self.show_title = show;
        self
    }

    /// Show the dialog. Pass `open = false` to start the close fade; keep calling every frame
    /// until `finished` is true, then drop your dialog state. Opening fades in the same way.
    pub fn show<R>(
        self,
        ctx: &Context,
        open: bool,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> DialogResponse<R> {
        let id = Id::new(("fuide-dialog", &self.title));
        let anim_id = id.with("anim");
        let now = ctx.input(|i| i.time);
        let mut anim = ctx.data(|d| d.get_temp::<Anim>(anim_id)).unwrap_or(Anim {
            opened_at: now,
            closing_at: None,
        });
        if !open && anim.closing_at.is_none() {
            anim.closing_at = Some(now);
        }
        let opacity = match anim.closing_at {
            None => ease_out(((now - anim.opened_at) / FADE_SECS).clamp(0.0, 1.0) as f32),
            Some(c) => 1.0 - ease_out(((now - c) / FADE_SECS).clamp(0.0, 1.0) as f32),
        };
        let finished = anim.closing_at.is_some_and(|c| now - c >= FADE_SECS);
        if finished {
            ctx.data_mut(|d| d.remove::<Anim>(anim_id));
            return DialogResponse {
                inner: None,
                should_close: false,
                finished: true,
            };
        }
        ctx.data_mut(|d| d.insert_temp(anim_id, anim));
        if opacity < 1.0 {
            ctx.request_repaint();
        }

        let pal = theme::palette(ctx);
        let ts = theme::type_scale(ctx);
        let c = theme::corners(ctx).panel;
        let outline = self.outline.unwrap_or(pal.accent);
        let width = self.width;
        let title = self.title.clone();
        let tag = self.tag.clone();
        let show_title = self.show_title;

        let modal = Modal::new(id)
            .area(Modal::default_area(id).fade_in(false)) // we drive the fade ourselves (both ways)
            .frame(Frame::NONE)
            .backdrop_color(pal.bg_deep.gamma_multiply(0.7 * opacity))
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                // reserve shapes below the content: body, then the chips on top afterwards
                let body_fill = ui.painter().add(Shape::Noop);
                let body_line = ui.painter().add(Shape::Noop);
                let glow: Vec<_> = (0..3).map(|_| ui.painter().add(Shape::Noop)).collect();

                ui.set_width(width);
                ui.add_space(PAD_Y);
                let inner = ui
                    .horizontal(|ui| {
                        ui.add_space(PAD_X);
                        ui.with_layout(Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(width - 2.0 * PAD_X);
                            add_contents(ui)
                        })
                        .inner
                    })
                    .inner;
                ui.add_space(PAD_Y);

                let rect =
                    Rect::from_min_size(ui.min_rect().min, vec2(width, ui.min_rect().height()));
                let pts = geom::hexagon_tl_br(rect, c);
                let p = ui.painter();
                p.set(
                    body_fill,
                    Shape::convex_polygon(pts.clone(), pal.bg_deep, egui::Stroke::NONE),
                );
                for (i, idx) in glow.iter().enumerate() {
                    let i = i as f32 + 1.0;
                    p.set(
                        *idx,
                        Shape::closed_line(
                            pts.clone(),
                            egui::Stroke::new(1.2 + 2.0 * i, outline.gamma_multiply(0.10 / i)),
                        ),
                    );
                }
                p.set(
                    body_line,
                    Shape::closed_line(pts, egui::Stroke::new(1.2, outline)),
                );

                let chip_h = (ts.label + 6.0).round();
                let inset = panel::CHIP_INSET.max(c);
                if show_title {
                    panel::chip(
                        p,
                        pos2(rect.left() + inset, rect.top() - chip_h / 2.0),
                        &title,
                        outline,
                        &pal,
                        ts.label,
                    );
                }
                if let Some((t, color)) = &tag {
                    let w = panel::chip_width(p, t, ts.label);
                    panel::chip(
                        p,
                        pos2(rect.right() - inset - w, rect.top() - chip_h / 2.0),
                        t,
                        *color,
                        &pal,
                        ts.label,
                    );
                }
                inner
            });
        DialogResponse {
            should_close: open && modal.should_close(),
            inner: Some(modal.inner),
            finished: false,
        }
    }
}

/// Right-aligned row of dialog buttons. `buttons` = (label, colour, enabled); returns the index clicked.
pub fn button_row(ui: &mut Ui, buttons: &[(&str, Color32, bool)]) -> Option<usize> {
    let ts = theme::type_scale(ui.ctx());
    let mut clicked = None;
    ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        for (i, (label, color, enabled)) in buttons.iter().enumerate().rev() {
            let w = ui
                .painter()
                .layout_no_wrap(label.to_string(), theme::mono(ts.data), *color)
                .size()
                .x
                + 28.0;
            if crate::widgets::button_colored(ui, vec2(w, ts.row + 2.0), label, *enabled, *color)
                .clicked()
            {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// Attention dialog: one big display-face word (`ERROR`, `WARNING`, `DONE`) in `color`, a short
/// mono line under it, a footnote pointing at the log, and a single `ACKNOWLEDGE` button.
/// Returns `inner == Some(true)` when acknowledged (button, Enter or Space).
pub fn alert(
    ctx: &Context,
    open: bool,
    word: &str,
    line: &str,
    footnote: &str,
    color: Color32,
) -> DialogResponse<bool> {
    let pal = theme::palette(ctx);
    let ts = theme::type_scale(ctx);
    let key =
        open && ctx.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space));
    Dialog::new(word)
        .outline(color)
        .show_title(false) // the body already says it, big
        .width(420.0)
        .show(ctx, open, |ui| {
            let big = ts.title * 2.2;
            let (r, _) = ui
                .allocate_exact_size(vec2(ui.available_width(), big + 16.0), egui::Sense::hover());
            theme::display_text(
                ui.painter(),
                r.center(),
                egui::Align2::CENTER_CENTER,
                word.to_uppercase(),
                big,
                color,
            );
            let (r, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), ts.data + 8.0),
                egui::Sense::hover(),
            );
            ui.painter().with_clip_rect(r).text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                line.to_uppercase(),
                theme::mono(ts.data),
                pal.text,
            );
            let (r, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), ts.label + 6.0),
                egui::Sense::hover(),
            );
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                footnote.to_uppercase(),
                theme::mono(ts.label),
                pal.text_dim,
            );
            ui.add_space(10.0);
            button_row(ui, &[("ACKNOWLEDGE", color, true)]) == Some(0) || key
        })
}
