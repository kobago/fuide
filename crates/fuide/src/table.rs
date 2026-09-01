//! Sortable column list: header row with whole-column hit areas, hover/selection rows, painter-drawn
//! cells (fast for thousands of rows), optional boxed "tag" cells. Data stays in the caller.

use egui::{pos2, vec2, Align2, Color32, Rect, Sense, Stroke, Ui};

use crate::{theme, widgets};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    /// Fixed width in points.
    Fixed(f32),
    /// Width for `n` characters of the label font plus padding.
    Chars(f32),
    /// Takes whatever is left (one column).
    Flex,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct Column {
    pub label: &'static str,
    pub width: Width,
    pub align: Align,
    pub sortable: bool,
}

impl Column {
    pub fn new(label: &'static str, width: Width) -> Self {
        Self {
            label,
            width,
            align: Align::Left,
            sortable: true,
        }
    }
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }
    pub fn unsortable(mut self) -> Self {
        self.sortable = false;
        self
    }
}

/// One rendered cell.
#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub text: String,
    pub color: Option<Color32>,
    /// Draw as a small boxed tag (`DIR`, `CASK`, `OUTDATED`).
    pub tag: bool,
    /// Data font (`true`) or the dimmer label font (`false`).
    pub primary: bool,
}

impl Cell {
    pub fn text(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            primary: true,
            ..Default::default()
        }
    }
    pub fn dim(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            ..Default::default()
        }
    }
    pub fn tag(s: impl Into<String>) -> Self {
        Self {
            text: s.into(),
            tag: true,
            ..Default::default()
        }
    }
    pub fn color(mut self, c: Color32) -> Self {
        self.color = Some(c);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct TableState {
    pub sort_col: usize,
    pub sort_desc: bool,
    pub selected: Option<usize>,
    /// Set to scroll the selected row into view on the next frame.
    pub scroll_to_selected: bool,
}

#[derive(Default)]
pub struct TableResponse {
    pub clicked: Option<usize>,
    pub double_clicked: Option<usize>,
    pub secondary_clicked: Option<usize>,
    pub sort_changed: bool,
}

/// Draw the table in the current `ui` (fills the available rect). `cell(row, col)` produces each cell;
/// rows are indices into the caller's (already sorted/filtered) view.
pub fn table(
    ui: &mut Ui,
    id_salt: &str,
    columns: &[Column],
    rows: usize,
    state: &mut TableState,
    mut cell: impl FnMut(usize, usize) -> Cell,
) -> TableResponse {
    let pal = theme::palette(ui.ctx());
    let ts = theme::type_scale(ui.ctx());
    let mut out = TableResponse::default();
    ui.spacing_mut().item_spacing.y = 0.0;
    let w = ui.available_width();
    let ch = ui
        .painter()
        .layout_no_wrap("0".into(), theme::mono(ts.label), pal.text)
        .size()
        .x;

    // column x ranges
    let mut fixed = 0.0;
    let mut widths: Vec<f32> = columns
        .iter()
        .map(|c| match c.width {
            Width::Fixed(px) => px,
            Width::Chars(n) => (ch * n + 14.0).round(),
            Width::Flex => 0.0,
        })
        .collect();
    for (c, wd) in columns.iter().zip(widths.iter()) {
        if c.width != Width::Flex {
            fixed += wd;
        }
    }
    for (c, wd) in columns.iter().zip(widths.iter_mut()) {
        if c.width == Width::Flex {
            *wd = (w - fixed).max(40.0);
        }
    }
    let mut xs = Vec::with_capacity(columns.len() + 1);
    let mut acc = 0.0;
    for wd in &widths {
        xs.push(acc);
        acc += wd;
    }
    xs.push(acc);

    // header
    {
        let (hr, _) = ui.allocate_exact_size(vec2(w, ts.heading + 8.0), Sense::hover());
        for (i, col) in columns.iter().enumerate() {
            let rect = Rect::from_min_max(
                pos2(hr.left() + xs[i], hr.top()),
                pos2(hr.left() + xs[i + 1], hr.bottom()),
            );
            let resp = if col.sortable {
                widgets::hit(ui, rect, (id_salt, "sort", i), Sense::click())
            } else {
                widgets::hit(ui, rect, (id_salt, "sort", i), Sense::hover())
            };
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, col.sortable, col.label)
            });
            let p = ui.painter();
            let active = col.sortable && state.sort_col == i;
            let hovered = col.sortable && resp.hovered();
            let color = if active || hovered {
                pal.accent
            } else {
                pal.text_dim
            };
            if hovered {
                p.rect_filled(
                    rect,
                    egui::CornerRadius::ZERO,
                    pal.accent.gamma_multiply(0.08),
                );
            }
            let tri_w = if active { 12.0 } else { 0.0 };
            let (x, anchor) = match col.align {
                Align::Left => (
                    rect.left() + if i == 0 { 8.0 } else { 4.0 },
                    Align2::LEFT_CENTER,
                ),
                Align::Right => (rect.right() - 8.0 - tri_w, Align2::RIGHT_CENTER),
            };
            let tr = theme::display_text(
                p,
                pos2(x, hr.center().y),
                anchor,
                col.label,
                ts.heading,
                color,
            );
            if active {
                let icon = if state.sort_desc {
                    widgets::Icon::TriangleDown
                } else {
                    widgets::Icon::TriangleUp
                };
                widgets::draw_icon(
                    p,
                    pos2(tr.right() + 8.0, hr.center().y),
                    6.0,
                    icon,
                    Stroke::new(1.0, color),
                );
            }
            if resp.clicked() {
                if state.sort_col == i {
                    state.sort_desc = !state.sort_desc;
                } else {
                    state.sort_col = i;
                    state.sort_desc = false;
                }
                out.sort_changed = true;
            }
        }
        ui.painter().hline(
            hr.x_range(),
            hr.bottom(),
            Stroke::new(1.0, pal.accent_dim.gamma_multiply(0.8)),
        );
    }

    // rows
    let row_h = ts.row;
    let scroll_to = state.scroll_to_selected.then_some(state.selected).flatten();
    state.scroll_to_selected = false;
    egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows, |ui, range| {
            let w = ui.available_width();
            for row in range {
                let (r, resp) = ui.allocate_exact_size(vec2(w, row_h), Sense::click());
                if scroll_to == Some(row) {
                    resp.scroll_to_me(None);
                }
                let is_sel = state.selected == Some(row);
                // rows are addressable by their first cell (the name column)
                let name = cell(row, 0).text;
                resp.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        is_sel,
                        name.clone(),
                    )
                });
                let p = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
                if is_sel {
                    p.rect_filled(r, egui::CornerRadius::ZERO, pal.accent.gamma_multiply(0.13));
                    p.rect_filled(
                        Rect::from_min_size(r.min, vec2(3.0, r.height())),
                        egui::CornerRadius::ZERO,
                        pal.accent,
                    );
                } else if resp.hovered() {
                    p.rect_filled(r, egui::CornerRadius::ZERO, pal.accent.gamma_multiply(0.05));
                } else if row % 2 == 1 {
                    p.rect_filled(r, egui::CornerRadius::ZERO, pal.accent.gamma_multiply(0.02));
                }
                let cy = r.center().y;
                for (i, col) in columns.iter().enumerate() {
                    let c = cell(row, i);
                    let cr = Rect::from_min_max(
                        pos2(r.left() + xs[i], r.top()),
                        pos2(r.left() + xs[i + 1], r.bottom()),
                    );
                    let pc = p.with_clip_rect(cr.intersect(p.clip_rect()));
                    if c.tag {
                        let tag_h = (ts.label + 5.0).round();
                        let tw = pc
                            .layout_no_wrap(c.text.clone(), theme::mono(ts.label), pal.text)
                            .size()
                            .x
                            + 12.0;
                        let tag_rect = Rect::from_min_size(
                            pos2(cr.left() + 2.0, cy - tag_h / 2.0),
                            vec2(tw.min(cr.width() - 4.0), tag_h),
                        );
                        let color = c.color.unwrap_or(pal.text_dim);
                        pc.rect_stroke(
                            tag_rect,
                            egui::CornerRadius::ZERO,
                            Stroke::new(1.0, color.gamma_multiply(0.7)),
                            egui::StrokeKind::Inside,
                        );
                        pc.text(
                            tag_rect.center(),
                            Align2::CENTER_CENTER,
                            &c.text,
                            theme::mono(ts.label),
                            color,
                        );
                    } else {
                        let (x, anchor) = match col.align {
                            Align::Left => (
                                cr.left() + if i == 0 { 8.0 } else { 4.0 },
                                Align2::LEFT_CENTER,
                            ),
                            Align::Right => (cr.right() - 8.0, Align2::RIGHT_CENTER),
                        };
                        let font = if c.primary {
                            theme::mono(ts.data)
                        } else {
                            theme::mono(ts.label)
                        };
                        let color =
                            c.color
                                .unwrap_or(if c.primary { pal.text } else { pal.text_dim });
                        pc.text(pos2(x, cy), anchor, &c.text, font, color);
                    }
                }
                if resp.double_clicked() {
                    out.double_clicked = Some(row);
                } else if resp.clicked() {
                    out.clicked = Some(row);
                    state.selected = Some(row);
                }
                if resp.secondary_clicked() {
                    out.secondary_clicked = Some(row);
                    state.selected = Some(row);
                }
            }
            if rows == 0 {
                let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                ui.painter().text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "NO ENTRIES",
                    theme::mono(ts.label),
                    pal.text_dim,
                );
            }
        });
    out
}
