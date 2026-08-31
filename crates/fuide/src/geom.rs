//! Polygons and glow strokes — the primitives every FUI part is built from.
//! Corners are square by default; the chamfer variants are opt-in (see `theme::Corners`).

use egui::{pos2, Color32, Painter, Pos2, Rect, Shape, Stroke};

/// Plain rectangle (the default corner treatment).
pub fn rect(r: Rect) -> Vec<Pos2> {
    vec![
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
    ]
}

/// Octagon: all four corners cut by `c`. `c <= 0` gives a plain rectangle.
pub fn octagon(r: Rect, c: f32) -> Vec<Pos2> {
    if c <= 0.0 {
        return rect(r);
    }
    vec![
        pos2(r.left() + c, r.top()),
        pos2(r.right() - c, r.top()),
        pos2(r.right(), r.top() + c),
        pos2(r.right(), r.bottom() - c),
        pos2(r.right() - c, r.bottom()),
        pos2(r.left() + c, r.bottom()),
        pos2(r.left(), r.bottom() - c),
        pos2(r.left(), r.top() + c),
    ]
}

/// Hexagon: top-left and bottom-right corners cut (optional panel shape). `c <= 0` = rectangle.
pub fn hexagon_tl_br(r: Rect, c: f32) -> Vec<Pos2> {
    if c <= 0.0 {
        return rect(r);
    }
    vec![
        pos2(r.left() + c, r.top()),
        pos2(r.right(), r.top()),
        pos2(r.right(), r.bottom() - c),
        pos2(r.right() - c, r.bottom()),
        pos2(r.left(), r.bottom()),
        pos2(r.left(), r.top() + c),
    ]
}

/// Pentagon: only the top-right corner cut (optional tab / button shape). `c <= 0` = rectangle.
pub fn pentagon_tr(r: Rect, c: f32) -> Vec<Pos2> {
    if c <= 0.0 {
        return rect(r);
    }
    vec![
        pos2(r.left(), r.top()),
        pos2(r.right() - c, r.top()),
        pos2(r.right(), r.top() + c),
        pos2(r.right(), r.bottom()),
        pos2(r.left(), r.bottom()),
    ]
}

/// Fill a convex polygon flat (no stroke).
pub fn fill(p: &Painter, pts: Vec<Pos2>, color: Color32) {
    p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Outline a closed polygon.
pub fn outline(p: &Painter, pts: Vec<Pos2>, stroke: Stroke) {
    p.add(Shape::closed_line(pts, stroke));
}

/// Glow = the same closed line drawn 4× with increasing width and decreasing alpha, then the core.
/// No blur available; this is how every "light" in the kit is made.
pub fn glow_outline(p: &Painter, pts: &[Pos2], color: Color32, core_width: f32, core_alpha: f32) {
    for i in 1..=4u32 {
        let w = core_width + 2.0 * i as f32;
        let a = 0.10 / i as f32;
        p.add(Shape::closed_line(
            pts.to_vec(),
            Stroke::new(w, color.gamma_multiply(a)),
        ));
    }
    p.add(Shape::closed_line(
        pts.to_vec(),
        Stroke::new(core_width, color.gamma_multiply(core_alpha)),
    ));
}

/// Glow for an open polyline (waveforms, sparklines).
pub fn glow_line(p: &Painter, pts: &[Pos2], color: Color32, core_width: f32, halo_width: f32) {
    p.add(Shape::line(
        pts.to_vec(),
        Stroke::new(halo_width, color.gamma_multiply(0.20)),
    ));
    p.add(Shape::line(pts.to_vec(), Stroke::new(core_width, color)));
}
