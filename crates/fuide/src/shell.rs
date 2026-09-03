//! Frameless window shell: body (square by default, chamfer optional), glow,
//! title bar with window buttons, status bar, scanline overlay and resize handles. Assumes
//! `ViewportBuilder::with_decorations(false).with_transparent(true)` and a transparent clear colour.

use egui::viewport::ResizeDirection as RD;
use egui::{
    pos2, vec2, Align2, Color32, CursorIcon, Id, Layout, Rect, Sense, Stroke, Ui, UiBuilder,
    ViewportCommand,
};

use crate::widgets::Icon;
use crate::{fx, geom, theme, widgets};

pub const TITLE_H: f32 = 40.0;
/// Idle animation rate. Override with `FUIDE_IDLE_FPS` (0 = every frame, for comparison).
pub const IDLE_FPS: u32 = 20;

fn idle_repaint_interval() -> std::time::Duration {
    static FPS: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    let fps = *FPS.get_or_init(|| {
        std::env::var("FUIDE_IDLE_FPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(IDLE_FPS)
    });
    if fps == 0 {
        std::time::Duration::ZERO
    } else {
        std::time::Duration::from_micros(1_000_000 / u64::from(fps))
    }
}
/// Horizontal inset of the title/status bars from the body edge (square corners).
pub const BAR_INSET: f32 = 14.0;

pub struct StatusLamp {
    pub text: String,
    pub color: Color32,
    pub blink: bool,
}

pub struct Shell {
    title: String,
    subtitle: String,
    status_left: String,
    lamps: Vec<StatusLamp>,
    tool_window: bool,
    settings_button: bool,
}

const KEEP_CLEAR_ID: &str = "fuide-shell-keep-clear";

/// Keep `rect` free of the shell's scanline / scan-band overlays for this frame (a video
/// picture, an image). Call it from inside the content closure; it applies to that frame only.
pub fn keep_clear(ctx: &egui::Context, rect: Rect) {
    let v = [rect.left(), rect.top(), rect.right(), rect.bottom()];
    ctx.data_mut(|d| d.insert_temp(Id::new(KEEP_CLEAR_ID), v));
}

/// What [`Shell::show_full`] hands back: the content closure's value plus title-bar clicks.
pub struct ShellOutput<R> {
    pub inner: R,
    /// The gear button (see [`Shell::settings_button`]) was clicked this frame.
    pub settings_clicked: bool,
    /// The window was asked to close this frame (× button or Cmd+W); `ViewportCommand::Close`
    /// has been sent. Apps can use it to flush state before the window goes.
    pub close_requested: bool,
}

impl Shell {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into().to_uppercase(),
            subtitle: String::new(),
            status_left: String::new(),
            lamps: Vec::new(),
            tool_window: false,
            settings_button: false,
        }
    }

    /// Small secondary window (settings, inspectors): only a close button, no resize handles,
    /// and no idle animation (it repaints on input only, so it never competes with the main
    /// window for frames). Use with a child viewport built with `with_resizable(false)`.
    pub fn tool_window(mut self) -> Self {
        self.tool_window = true;
        self
    }

    /// Show a gear button left of the window buttons. Read the click via [`Shell::show_full`].
    pub fn settings_button(mut self, show: bool) -> Self {
        self.settings_button = show;
        self
    }

    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = s.into().to_uppercase();
        self
    }

    /// Status text (not upper-cased: units like `T+00:12:34.5` keep their case).
    pub fn status_left(mut self, s: impl Into<String>) -> Self {
        self.status_left = s.into();
        self
    }

    pub fn lamp(mut self, text: impl Into<String>, color: Color32, blink: bool) -> Self {
        self.lamps.push(StatusLamp {
            text: text.into(),
            color,
            blink,
        });
        self
    }

    /// Draw the shell over the whole root `ui` and run `add_contents` in the content area.
    pub fn show<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
        self.show_full(ui, add_contents).inner
    }

    /// Like [`Shell::show`] but also reports title-bar button clicks.
    pub fn show_full<R>(
        self,
        ui: &mut Ui,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> ShellOutput<R> {
        // Ids are salted with the viewport *and* the root `Ui` so a tool window drawn with its own
        // shell never shares interaction state with the main window's shell — neither as a real
        // child viewport nor when egui embeds it as a `Window` (`embed_viewports`, where both
        // shells run in the same viewport).
        let vp = ui.id().with(ui.ctx().viewport_id());
        // Cmd+W closes the window (macOS convention). For the main shell that ends the app;
        // it works while a dialog is up or a text field has focus, unlike app shortcuts.
        // Tool windows close themselves (their host also sees the key when embedded).
        let mut close_requested =
            !self.tool_window && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::W));
        if close_requested {
            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
        }
        let pal = theme::palette(ui.ctx());
        let chamfer = theme::corners(ui.ctx()).window;
        let ts = theme::type_scale(ui.ctx());
        let inset = BAR_INSET.max(chamfer);
        let t = ui.input(|i| i.time);
        let full = ui.max_rect();
        let r = full.shrink(3.0);

        // ---- body + glow (pulsing core) ------------------------------------------------
        {
            let p = ui.painter();
            let pts = geom::octagon(r, chamfer);
            geom::fill(p, pts.clone(), pal.bg_deep);
            let pulse = if self.tool_window {
                0.85
            } else {
                0.75 + 0.25 * (2.0 * t).sin() as f32
            };
            geom::glow_outline(p, &pts, pal.accent, 1.4, pulse);
        }

        // ---- title bar --------------------------------------------------------------------
        let bar = Rect::from_min_max(
            pos2(r.left() + inset, r.top() + 6.0),
            pos2(r.right() - inset, r.top() + 6.0 + TITLE_H),
        );
        // drag region first; buttons registered later win the overlap
        let drag = ui.interact(
            bar,
            Id::new(("fuide-shell-drag", vp)),
            Sense::click_and_drag(),
        );
        if drag.drag_started() {
            ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() && !self.tool_window {
            let maxed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::Maximized(!maxed));
        }
        {
            let p = ui.painter();
            let mut x = bar.left() + 10.0;
            let cy = bar.center().y;
            let tr = theme::display_text(
                p,
                pos2(x, cy),
                Align2::LEFT_CENTER,
                &self.title,
                ts.title,
                pal.accent,
            );
            x = tr.right() + 14.0;
            if !self.subtitle.is_empty() {
                p.text(
                    pos2(x, cy + 1.0),
                    Align2::LEFT_CENTER,
                    &self.subtitle,
                    theme::mono(ts.label),
                    pal.text_dim,
                );
            }
            p.hline(
                (r.left() + 14.0)..=(r.right() - 14.0),
                bar.bottom(),
                Stroke::new(1.0, pal.accent_dim),
            );
        }
        // window buttons: close / max / min from the right (tool windows: close only)
        let mut settings_clicked = false;
        {
            let cy = bar.center().y;
            let mut cx = bar.right() - 10.0 - 15.0;
            let buttons: &[(Glyph, Color32)] = if self.tool_window {
                &[(Glyph::Close, pal.danger)]
            } else {
                &[
                    (Glyph::Close, pal.danger),
                    (Glyph::Max, pal.accent),
                    (Glyph::Min, pal.accent),
                ]
            };
            for &(glyph, color) in buttons {
                let brect = Rect::from_center_size(pos2(cx, cy), vec2(30.0, 26.0));
                let resp = ui.interact(
                    brect,
                    Id::new(("fuide-winbtn", glyph as u8, vp)),
                    Sense::click(),
                );
                crate::agent::describe(&resp, || {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, glyph.label())
                });
                let p = ui.painter();
                // glyph is always full-strength; hover only lights the background
                if resp.hovered() {
                    p.rect_filled(brect, egui::CornerRadius::ZERO, color.gamma_multiply(0.22));
                }
                let s = Stroke::new(1.5, color);
                let c = brect.center();
                let h = 5.0;
                match glyph {
                    Glyph::Close => {
                        p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], s);
                        p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], s);
                    }
                    Glyph::Max => {
                        p.rect_stroke(
                            Rect::from_center_size(c, vec2(10.0, 10.0)),
                            egui::CornerRadius::ZERO,
                            s,
                            egui::StrokeKind::Middle,
                        );
                    }
                    Glyph::Min => {
                        p.line_segment([c + vec2(-h, 3.0), c + vec2(h, 3.0)], s);
                    }
                }
                if resp.clicked() {
                    match glyph {
                        Glyph::Close => {
                            close_requested = true;
                            ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                        }
                        Glyph::Max => {
                            let maxed = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                            ui.ctx()
                                .send_viewport_cmd(ViewportCommand::Maximized(!maxed));
                        }
                        Glyph::Min => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
                    }
                }
                cx -= 38.0;
            }
            if self.settings_button {
                // gear, a step further from the window buttons so it does not read as one of them
                cx -= 8.0;
                let brect = Rect::from_center_size(pos2(cx, cy), vec2(30.0, 26.0));
                let resp = ui.interact(
                    brect,
                    Id::new(("fuide-winbtn-settings", vp)),
                    Sense::click(),
                );
                crate::agent::describe(&resp, || {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        true,
                        Icon::Settings.label(),
                    )
                });
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(
                        brect,
                        egui::CornerRadius::ZERO,
                        pal.accent.gamma_multiply(0.22),
                    );
                }
                widgets::draw_icon(
                    p,
                    brect.center(),
                    13.0,
                    Icon::Settings,
                    Stroke::new(1.5, pal.accent),
                );
                settings_clicked = resp.clicked();
            }
        }

        // ---- status bar -------------------------------------------------------------------
        {
            let p = ui.painter();
            p.hline(
                (r.left() + 14.0)..=(r.right() - 14.0),
                r.bottom() - 28.0,
                Stroke::new(1.0, pal.accent_dim.gamma_multiply(0.6)),
            );
            let y = r.bottom() - 16.0;
            p.text(
                pos2(r.left() + inset + 4.0, y),
                Align2::LEFT_CENTER,
                &self.status_left,
                theme::mono(ts.label),
                pal.text_dim,
            );
            // lamps, right-aligned, laid out right→left
            let mut x = r.right() - inset - 4.0;
            for l in self.lamps.iter().rev() {
                let w = p
                    .layout_no_wrap(l.text.to_uppercase(), theme::mono(ts.label), pal.text_dim)
                    .size()
                    .x
                    + 10.0;
                x -= w;
                widgets::lamp(
                    p,
                    pos2(x, y),
                    l.color,
                    &l.text,
                    l.blink,
                    t,
                    pal.text_dim,
                    ts.label,
                );
                x -= 18.0;
            }
        }

        // ---- content ----------------------------------------------------------------------
        let content = Rect::from_min_max(
            pos2(r.left() + 14.0, bar.bottom() + 14.0),
            pos2(r.right() - 14.0, r.bottom() - 36.0),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("fuide-shell-content")
                .max_rect(content)
                .layout(Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(content);
        let out = add_contents(&mut child);

        // ---- overlays + resize handles ----------------------------------------------------
        // (a rectangle registered with `keep_clear` — a video picture — gets no scanlines)
        let hole = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<[f32; 4]>(Id::new(KEEP_CLEAR_ID)))
            .map(|[l, t, r, b]| Rect::from_min_max(pos2(l, t), pos2(r, b)));
        let regions: Vec<Rect> = match hole {
            None => vec![r],
            Some(h) => {
                let h = h.intersect(r);
                vec![
                    Rect::from_min_max(r.min, pos2(r.right(), h.top())),
                    Rect::from_min_max(pos2(r.left(), h.bottom()), r.max),
                    Rect::from_min_max(pos2(r.left(), h.top()), pos2(h.left(), h.bottom())),
                    Rect::from_min_max(pos2(h.right(), h.top()), pos2(r.right(), h.bottom())),
                ]
            }
        };
        for region in regions.into_iter().filter(|q| q.is_positive()) {
            let p = ui
                .painter()
                .with_clip_rect(region.intersect(ui.clip_rect()));
            if !self.tool_window {
                fx::scan_band(&p, r, t, pal.accent);
            }
            fx::scanlines(&p, r);
        }
        if !self.tool_window {
            resize_handles(ui, full, vp);
        }
        if !self.tool_window {
            // The shell is always animating (pulse + band), but repainting every vsync makes macOS
            // window managers (Rectangle) lag 200-500 ms on snap resizes — winit #3644. Idle
            // animation at ~20 fps is indistinguishable for slow motion and keeps the event queue
            // empty. Tool windows do not animate at all: they repaint on input only.
            ui.ctx().request_repaint_after(idle_repaint_interval());
        }
        ShellOutput {
            inner: out,
            settings_clicked,
            close_requested,
        }
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Close = 0,
    Max = 1,
    Min = 2,
}

impl Glyph {
    fn label(self) -> &'static str {
        match self {
            Self::Close => "CLOSE WINDOW",
            Self::Max => "MAXIMIZE",
            Self::Min => "MINIMIZE",
        }
    }
}

fn resize_handles(ui: &mut Ui, rect: Rect, vp: Id) {
    let m = 7.0;
    let edges: [(RD, Rect, CursorIcon); 8] = [
        (
            RD::East,
            Rect::from_min_max(
                pos2(rect.right() - m, rect.top() + m),
                pos2(rect.right(), rect.bottom() - m),
            ),
            CursorIcon::ResizeEast,
        ),
        (
            RD::West,
            Rect::from_min_max(
                pos2(rect.left(), rect.top() + m),
                pos2(rect.left() + m, rect.bottom() - m),
            ),
            CursorIcon::ResizeWest,
        ),
        (
            RD::North,
            Rect::from_min_max(
                pos2(rect.left() + m, rect.top()),
                pos2(rect.right() - m, rect.top() + m),
            ),
            CursorIcon::ResizeNorth,
        ),
        (
            RD::South,
            Rect::from_min_max(
                pos2(rect.left() + m, rect.bottom() - m),
                pos2(rect.right() - m, rect.bottom()),
            ),
            CursorIcon::ResizeSouth,
        ),
        // corners after edges: later registration wins the overlap
        (
            RD::NorthWest,
            Rect::from_min_size(rect.min, vec2(2.0 * m, 2.0 * m)),
            CursorIcon::ResizeNorthWest,
        ),
        (
            RD::NorthEast,
            Rect::from_min_size(
                pos2(rect.right() - 2.0 * m, rect.top()),
                vec2(2.0 * m, 2.0 * m),
            ),
            CursorIcon::ResizeNorthEast,
        ),
        (
            RD::SouthWest,
            Rect::from_min_size(
                pos2(rect.left(), rect.bottom() - 2.0 * m),
                vec2(2.0 * m, 2.0 * m),
            ),
            CursorIcon::ResizeSouthWest,
        ),
        (
            RD::SouthEast,
            Rect::from_min_size(
                pos2(rect.right() - 2.0 * m, rect.bottom() - 2.0 * m),
                vec2(2.0 * m, 2.0 * m),
            ),
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (dir, hrect, icon) in edges {
        let resp = ui.interact(
            hrect,
            Id::new(("fuide-resize", icon as u32, vp)),
            Sense::drag(),
        );
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(icon);
        }
        if resp.drag_started() {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
