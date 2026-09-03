//! Instrument-style parts: nav tab, button, segment bar, arc gauge, lamp, log feed.

use egui::{
    pos2, vec2, Align2, Color32, Id, Painter, Pos2, Rect, Response, Sense, Stroke, Ui, WidgetInfo,
    WidgetType,
};

use crate::{geom, theme};

// ---------------------------------------------------------------------------
// Nav tab (sidebar entry)

/// Sidebar tab with a 3px bar on the left. Width fills the row. (Optional top-right chamfer.)
pub fn nav_tab(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let pal = theme::palette(ui.ctx());
    let c = theme::corners(ui.ctx()).tab;
    let ts = theme::type_scale(ui.ctx());
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ts.row + 2.0), Sense::click());
    // accessibility label = what is drawn (upper-cased), so tests and screen readers agree
    crate::agent::describe(&resp, || {
        WidgetInfo::selected(
            WidgetType::SelectableLabel,
            true,
            selected,
            label.to_uppercase(),
        )
    });
    let p = ui.painter();

    let fill = if selected {
        pal.accent.gamma_multiply(0.13)
    } else {
        pal.bg_panel.gamma_multiply(0.5)
    };
    let a = if selected {
        1.0
    } else if resp.hovered() {
        0.55
    } else {
        0.22
    };
    let pts = geom::pentagon_tr(rect, c);
    geom::fill(p, pts.clone(), fill);
    geom::outline(p, pts, Stroke::new(1.0, pal.accent.gamma_multiply(a)));
    let bar = Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
    p.rect_filled(
        bar,
        egui::CornerRadius::ZERO,
        if selected {
            pal.accent
        } else {
            pal.accent_dim.gamma_multiply(0.4)
        },
    );
    theme::display_text(
        p,
        pos2(rect.left() + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label.to_uppercase(),
        ts.heading,
        if selected {
            pal.accent
        } else {
            pal.text.gamma_multiply(0.8)
        },
    );
    resp
}

/// Small section label above a group (`MODULES`, `VOLUMES`).
pub fn section_label(ui: &mut Ui, label: &str) {
    let pal = theme::palette(ui.ctx());
    let ts = theme::type_scale(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), ts.heading + 6.0), Sense::hover());
    theme::display_text(
        ui.painter(),
        pos2(rect.left() + 2.0, rect.center().y),
        Align2::LEFT_CENTER,
        label.to_uppercase(),
        ts.heading,
        pal.text_dim,
    );
}

// ---------------------------------------------------------------------------
// Chamfer button (toolbar)

/// Outlined text button in the accent colour. `label` is mono text (ASCII glyphs only).
pub fn button(ui: &mut Ui, size: egui::Vec2, label: &str, enabled: bool) -> Response {
    let accent = theme::palette(ui.ctx()).accent;
    button_colored(ui, size, label, enabled, accent)
}

/// Outlined text button in an explicit colour (use `warn` / `danger` for consequential actions).
pub fn button_colored(
    ui: &mut Ui,
    size: egui::Vec2,
    label: &str,
    enabled: bool,
    color: Color32,
) -> Response {
    let pal = theme::palette(ui.ctx());
    let c = theme::corners(ui.ctx()).button;
    let ts = theme::type_scale(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    crate::agent::describe(&resp, || {
        WidgetInfo::labeled(WidgetType::Button, enabled, label)
    });
    let p = ui.painter();
    let a = if !enabled {
        0.15
    } else if resp.is_pointer_button_down_on() {
        1.0
    } else if resp.hovered() {
        0.6
    } else {
        0.28
    };
    let pts = geom::pentagon_tr(rect, c);
    if enabled && resp.hovered() {
        geom::fill(p, pts.clone(), color.gamma_multiply(0.12));
    }
    geom::outline(p, pts, Stroke::new(1.0, color.gamma_multiply(a)));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        theme::mono(ts.data),
        if enabled {
            color.gamma_multiply(0.4 + 0.6 * a)
        } else {
            pal.text_dim.gamma_multiply(0.5)
        },
    );
    resp
}

// ---------------------------------------------------------------------------
// Line icons (never glyphs from a font — those fall back to `?` or look like text)

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    ChevronDown,
    ArrowUp,
    ArrowDown,
    TriangleUp,
    TriangleDown,
    Plus,
    Cross,
    /// Circular arrow (reload).
    Refresh,
    /// Gear (settings): ring with radial teeth.
    Settings,
    /// Transport: filled triangle.
    Play,
    Pause,
    Stop,
    /// Transport: bar + triangle (previous / next track).
    SkipBack,
    SkipForward,
}

impl Icon {
    /// Accessibility label (what a screen reader / UI test sees for an icon-only button).
    pub fn label(self) -> &'static str {
        match self {
            Self::ChevronLeft => "BACK",
            Self::ChevronRight => "FORWARD",
            Self::ChevronUp => "UP",
            Self::ChevronDown => "DOWN",
            Self::ArrowUp => "ARROW UP",
            Self::ArrowDown => "ARROW DOWN",
            Self::TriangleUp => "ASCENDING",
            Self::TriangleDown => "DESCENDING",
            Self::Plus => "ADD",
            Self::Cross => "CLOSE",
            Self::Refresh => "REFRESH",
            Self::Settings => "SETTINGS",
            Self::Play => "PLAY",
            Self::Pause => "PAUSE",
            Self::Stop => "STOP",
            Self::SkipBack => "PREVIOUS",
            Self::SkipForward => "NEXT",
        }
    }
}

/// Draw `icon` centred on `center`, fitting a `size` × `size` box, with `stroke`.
pub fn draw_icon(p: &Painter, center: Pos2, size: f32, icon: Icon, stroke: Stroke) {
    let h = size / 2.0;
    let q = size / 4.0;
    let c = center;
    let chevron = |pts: [Pos2; 3]| p.add(egui::Shape::line(pts.to_vec(), stroke));
    match icon {
        Icon::ChevronLeft => {
            chevron([c + vec2(q, -h), c + vec2(-q, 0.0), c + vec2(q, h)]);
        }
        Icon::ChevronRight => {
            chevron([c + vec2(-q, -h), c + vec2(q, 0.0), c + vec2(-q, h)]);
        }
        Icon::ChevronUp => {
            chevron([c + vec2(-h, q), c + vec2(0.0, -q), c + vec2(h, q)]);
        }
        Icon::ChevronDown => {
            chevron([c + vec2(-h, -q), c + vec2(0.0, q), c + vec2(h, -q)]);
        }
        Icon::ArrowUp => {
            chevron([c + vec2(-q, -q), c + vec2(0.0, -h), c + vec2(q, -q)]);
            p.line_segment([c + vec2(0.0, -h), c + vec2(0.0, h)], stroke);
        }
        Icon::ArrowDown => {
            chevron([c + vec2(-q, q), c + vec2(0.0, h), c + vec2(q, q)]);
            p.line_segment([c + vec2(0.0, -h), c + vec2(0.0, h)], stroke);
        }
        Icon::TriangleUp => {
            p.add(egui::Shape::convex_polygon(
                vec![c + vec2(-h, q), c + vec2(0.0, -q), c + vec2(h, q)],
                stroke.color,
                Stroke::NONE,
            ));
        }
        Icon::TriangleDown => {
            p.add(egui::Shape::convex_polygon(
                vec![c + vec2(-h, -q), c + vec2(h, -q), c + vec2(0.0, q)],
                stroke.color,
                Stroke::NONE,
            ));
        }
        Icon::Plus => {
            p.line_segment([c + vec2(-h, 0.0), c + vec2(h, 0.0)], stroke);
            p.line_segment([c + vec2(0.0, -h), c + vec2(0.0, h)], stroke);
        }
        Icon::Cross => {
            p.line_segment([c + vec2(-h, -h), c + vec2(h, h)], stroke);
            p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], stroke);
        }
        Icon::Refresh => {
            // 300° arc with an arrowhead at its end
            let r = h;
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let a = (-60.0 + 300.0 * i as f32 / 24.0).to_radians();
                    c + vec2(r * a.cos(), r * a.sin())
                })
                .collect();
            let end = *pts.last().unwrap();
            p.add(egui::Shape::line(pts, stroke));
            let a = (240.0f32).to_radians();
            let tangent = vec2(-a.sin(), a.cos());
            let normal = vec2(a.cos(), a.sin());
            let s = q * 0.9;
            p.line_segment([end, end - tangent * s + normal * s], stroke);
            p.line_segment([end, end - tangent * s - normal * s], stroke);
        }
        Icon::Settings => {
            // ring + 8 radial teeth
            let r = h * 0.55;
            p.circle_stroke(c, r, stroke);
            for i in 0..8 {
                let a = (45.0 * i as f32).to_radians();
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * (r + 1.0), c + d * h], stroke);
            }
        }
        Icon::Play => {
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-q * 0.8, -h),
                    c + vec2(h, 0.0),
                    c + vec2(-q * 0.8, h),
                ],
                stroke.color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            let w = q * 0.7;
            p.rect_filled(
                Rect::from_min_max(c + vec2(-h * 0.8, -h), c + vec2(-h * 0.8 + w, h)),
                egui::CornerRadius::ZERO,
                stroke.color,
            );
            p.rect_filled(
                Rect::from_min_max(c + vec2(h * 0.8 - w, -h), c + vec2(h * 0.8, h)),
                egui::CornerRadius::ZERO,
                stroke.color,
            );
        }
        Icon::Stop => {
            p.rect_filled(
                Rect::from_center_size(c, vec2(size * 0.85, size * 0.85)),
                egui::CornerRadius::ZERO,
                stroke.color,
            );
        }
        Icon::SkipBack | Icon::SkipForward => {
            // bar at the destination side, triangle pointing at it
            let dir = if icon == Icon::SkipForward { 1.0 } else { -1.0 };
            let bar_x = dir * h;
            p.line_segment(
                [c + vec2(bar_x, -h * 0.9), c + vec2(bar_x, h * 0.9)],
                Stroke::new(stroke.width * 1.4, stroke.color),
            );
            p.add(egui::Shape::convex_polygon(
                vec![
                    c + vec2(-dir * h, -h * 0.9),
                    c + vec2(dir * (h - q * 0.6), 0.0),
                    c + vec2(-dir * h, h * 0.9),
                ],
                stroke.color,
                Stroke::NONE,
            ));
        }
    }
}

/// Outlined button showing a line icon instead of text (same states as [`button`]).
pub fn icon_button(ui: &mut Ui, size: egui::Vec2, icon: Icon, enabled: bool) -> Response {
    let pal = theme::palette(ui.ctx());
    let c = theme::corners(ui.ctx()).button;
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    crate::agent::describe(&resp, || {
        WidgetInfo::labeled(WidgetType::Button, enabled, icon.label())
    });
    let p = ui.painter();
    let a = if !enabled {
        0.15
    } else if resp.is_pointer_button_down_on() {
        1.0
    } else if resp.hovered() {
        0.6
    } else {
        0.28
    };
    let pts = geom::pentagon_tr(rect, c);
    if enabled && resp.hovered() {
        geom::fill(p, pts.clone(), pal.accent.gamma_multiply(0.12));
    }
    geom::outline(p, pts, Stroke::new(1.0, pal.accent.gamma_multiply(a)));
    let color = if enabled {
        pal.accent.gamma_multiply(0.4 + 0.6 * a)
    } else {
        pal.text_dim.gamma_multiply(0.5)
    };
    draw_icon(
        p,
        rect.center(),
        (rect.height() * 0.42).round(),
        icon,
        Stroke::new(1.5, color),
    );
    resp
}

/// Framed single-line text input (`FILTER`, rename fields). Returns the `TextEdit` response.
pub fn text_input(ui: &mut Ui, width: f32, text: &mut String, hint: &str) -> Response {
    let pal = theme::palette(ui.ctx());
    let c = theme::corners(ui.ctx()).button;
    let ts = theme::type_scale(ui.ctx());
    let (frame_rect, _) = ui.allocate_exact_size(vec2(width, ts.row), Sense::hover());
    let pts = geom::pentagon_tr(frame_rect, c);
    geom::fill(ui.painter(), pts.clone(), pal.bg_deep.gamma_multiply(0.6));
    let outline_idx = ui.painter().add(egui::Shape::Noop);
    let inner = frame_rect.shrink2(vec2(6.0, 2.0));
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .frame(egui::Frame::NONE)
            .font(theme::mono(ts.data))
            .text_color(pal.accent)
            .hint_text(
                egui::RichText::new(hint.to_uppercase())
                    .font(theme::mono(ts.label))
                    .color(pal.text_dim),
            )
            .desired_width(f32::INFINITY),
    );
    // the TextEdit reports its own accesskit node without a label: re-describe it so the
    // agent and UI tests find it under the hint text
    let mut info = WidgetInfo::text_edit(true, text.as_str(), text.as_str(), hint);
    info.label = Some(hint.to_uppercase());
    crate::agent::describe(&resp, || info);
    let a = if resp.has_focus() { 0.9 } else { 0.35 };
    ui.painter().set(
        outline_idx,
        egui::Shape::closed_line(pts, Stroke::new(1.0, pal.accent.gamma_multiply(a))),
    );
    resp
}

/// Text toggle chip (`HIDDEN` lit / unlit), drawn like [`button`] with state colour.
pub fn toggle_chip(ui: &mut Ui, label: &str, on: &mut bool) -> Response {
    let pal = theme::palette(ui.ctx());
    let c = theme::corners(ui.ctx()).button;
    let ts = theme::type_scale(ui.ctx());
    let w = ui
        .painter()
        .layout_no_wrap(label.to_uppercase(), theme::mono(ts.label), pal.text)
        .size()
        .x
        + 24.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, ts.row), Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    crate::agent::describe(&resp, || {
        WidgetInfo::selected(WidgetType::Checkbox, true, *on, label.to_uppercase())
    });
    let p = ui.painter();
    let pts = geom::pentagon_tr(rect, c);
    if *on {
        geom::fill(p, pts.clone(), pal.accent.gamma_multiply(0.13));
    }
    let a = if *on {
        0.9
    } else if resp.hovered() {
        0.55
    } else {
        0.25
    };
    geom::outline(p, pts, Stroke::new(1.0, pal.accent.gamma_multiply(a)));
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label.to_uppercase(),
        theme::mono(ts.label),
        if *on { pal.accent } else { pal.text_dim },
    );
    resp
}

// ---------------------------------------------------------------------------
// Segment bar

/// 18 LED-like segments. `value` 0..1. Lit segments brighten toward the right.
pub fn segment_bar(p: &Painter, rect: Rect, value: f32, color: Color32, dim: Color32) {
    const N: usize = 18;
    let gap = 2.0;
    let w = (rect.width() - gap * (N as f32 - 1.0)) / N as f32;
    for i in 0..N {
        let f = i as f32 / (N as f32 - 1.0);
        let x = rect.left() + i as f32 * (w + gap);
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(w, rect.height()));
        let c = if f <= value.clamp(0.0, 1.0) {
            color.gamma_multiply(0.25 + 0.75 * f)
        } else {
            dim.gamma_multiply(0.15)
        };
        p.rect_filled(r, egui::CornerRadius::ZERO, c);
    }
}

// ---------------------------------------------------------------------------
// Arc gauge

/// 270° arc gauge, opening at the bottom. Percent in the centre, label below.
pub fn arc_gauge(ui: &mut Ui, radius: f32, value: f32, label: &str, color: Color32) -> Response {
    let pal = theme::palette(ui.ctx());
    let ts = theme::type_scale(ui.ctx());
    let size = vec2(radius * 2.0 + 16.0, radius * 2.0 + 30.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let center = pos2(rect.center().x, rect.top() + radius + 8.0);
    let p = ui.painter();

    let arc = |from: f32, to: f32| -> Vec<Pos2> {
        let n = 40;
        (0..=n)
            .map(|i| {
                let f = from + (to - from) * i as f32 / n as f32;
                let a = (135.0 + 270.0 * f).to_radians();
                pos2(center.x + radius * a.cos(), center.y + radius * a.sin())
            })
            .collect()
    };
    p.add(egui::Shape::line(
        arc(0.0, 1.0),
        Stroke::new(3.0, pal.accent_dim.gamma_multiply(0.35)),
    ));
    let v = value.clamp(0.0, 1.0);
    if v > 0.0 {
        p.add(egui::Shape::line(
            arc(0.0, v),
            Stroke::new(5.0, color.gamma_multiply(0.35)),
        ));
        p.add(egui::Shape::line(arc(0.0, v), Stroke::new(3.0, color)));
    }
    theme::display_text(
        p,
        center,
        Align2::CENTER_CENTER,
        format!("{:.0}", v * 100.0),
        radius * 0.5,
        color,
    );
    p.text(
        pos2(center.x, center.y + radius + 12.0),
        Align2::CENTER_CENTER,
        label.to_uppercase(),
        theme::mono(ts.label),
        pal.text_dim,
    );
    resp
}

// ---------------------------------------------------------------------------
// Status lamp

/// r=3 dot + noun. `blink` = 1.5 Hz square wave (use only for in-progress states).
#[allow(clippy::too_many_arguments)]
pub fn lamp(
    p: &Painter,
    pos: Pos2,
    color: Color32,
    text: &str,
    blink: bool,
    t: f64,
    text_color: Color32,
    size: f32,
) -> f32 {
    let a = if blink {
        if (t * 1.5).fract() < 0.5 {
            1.0
        } else {
            0.3
        }
    } else {
        1.0
    };
    p.circle_filled(pos, 3.0, color.gamma_multiply(a));
    let r = p.text(
        pos2(pos.x + 10.0, pos.y),
        Align2::LEFT_CENTER,
        text.to_uppercase(),
        theme::mono(size),
        text_color,
    );
    r.right() - pos.x
}

// ---------------------------------------------------------------------------
// Log feed

pub struct LogLine {
    pub time: String,
    pub text: String,
    pub color: Color32,
}

/// Row order of a [`log_feed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOrder {
    /// Telemetry style: newest at the top, older rows fade.
    NewestFirst,
    /// Terminal style: chronological, view sticks to the bottom while new lines arrive.
    Chronological,
}

/// Long lines wrap, text is selectable (drag + Cmd/Ctrl+C), scrolls when full.
/// `NewestFirst` fades older rows to 55 %; `Chronological` keeps every row full strength.
pub fn log_feed(ui: &mut Ui, lines: &[LogLine], time_color: Color32, size: f32, order: LogOrder) {
    let time_w = lines
        .iter()
        .rev()
        .take(64)
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.time.clone(), theme::mono(size), time_color)
                .size()
                .x
        })
        .fold(0.0_f32, f32::max)
        + 10.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("fuide-log-feed")
        .auto_shrink([false, false]);
    if order == LogOrder::Chronological {
        area = area.stick_to_bottom(true);
    }
    area.show(ui, |ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 2.0);
        ui.style_mut().interaction.selectable_labels = true;
        ui.style_mut().interaction.multi_widget_text_select = true;
        let row = |ui: &mut Ui, line: &LogLine, a: f32| {
            ui.horizontal_top(|ui| {
                ui.add_sized(
                    [time_w, size + 4.0],
                    egui::Label::new(
                        egui::RichText::new(&line.time)
                            .font(theme::mono(size))
                            .color(time_color.gamma_multiply(a)),
                    )
                    .extend(),
                );
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&line.text)
                            .font(theme::mono(size))
                            .color(line.color.gamma_multiply(a)),
                    )
                    .wrap(),
                );
            });
        };
        match order {
            LogOrder::NewestFirst => {
                for (i, line) in lines.iter().rev().enumerate() {
                    row(ui, line, (1.0 - i as f32 * 0.075).max(0.55));
                }
            }
            LogOrder::Chronological => {
                for line in lines {
                    row(ui, line, 1.0);
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Key / value readout line (`NAME ...... VALUE`)

pub fn readout(ui: &mut Ui, key: &str, value: &str, value_color: Option<Color32>) {
    let pal = theme::palette(ui.ctx());
    let ts = theme::type_scale(ui.ctx());
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), ts.data + 6.0), Sense::hover());
    let p = ui.painter().with_clip_rect(rect);
    p.text(
        pos2(rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        key.to_uppercase(),
        theme::mono(ts.label),
        pal.text_dim,
    );
    p.text(
        pos2(rect.right(), rect.center().y),
        Align2::RIGHT_CENTER,
        value,
        theme::mono(ts.data),
        value_color.unwrap_or(pal.text),
    );
}

/// Thin separator line in accent_dim.
pub fn rule(ui: &mut Ui) {
    let pal = theme::palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, pal.accent_dim.gamma_multiply(0.5)),
    );
}

// ---------------------------------------------------------------------------
// Splitter

/// Horizontal divider between two stacked regions: `strip` is the gap between them. Dragging it
/// up grows `value` (the height of the region *below*), clamped to `min..=max`. Drawn as a short
/// centred grip that brightens on hover; the cursor becomes a vertical resize arrow. `label` is
/// the accessibility name (e.g. `LOG HEIGHT`). Read `drag_stopped()` to persist the value.
pub fn h_splitter(
    ui: &mut Ui,
    strip: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut f32,
    min: f32,
    max: f32,
    label: &str,
) -> Response {
    let pal = theme::palette(ui.ctx());
    let max = max.max(min);
    let resp = hit(ui, strip, ("h-splitter", salt), Sense::drag());
    if resp.dragged() {
        *value = (*value - resp.drag_delta().y).clamp(min, max);
    }
    *value = value.clamp(min, max);
    crate::agent::describe(&resp, || WidgetInfo::slider(true, f64::from(*value), label));
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let a = if resp.dragged() {
        1.0
    } else if resp.hovered() {
        0.7
    } else {
        0.3
    };
    let p = ui.painter();
    let c = strip.center();
    for dy in [-2.0, 2.0] {
        p.hline(
            (c.x - 18.0)..=(c.x + 18.0),
            c.y + dy,
            Stroke::new(1.0, pal.accent.gamma_multiply(a)),
        );
    }
    resp
}

/// Register an invisible interact region (used by callers that need a Response over a painted area).
pub fn hit(
    ui: &Ui,
    rect: Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
    sense: Sense,
) -> Response {
    ui.interact(rect, Id::new(("fuide-hit", salt)), sense)
}
