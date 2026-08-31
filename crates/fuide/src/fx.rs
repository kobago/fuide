//! Full-screen overlays: scanlines and the slow scan band.

use egui::{pos2, Color32, Painter, Rect, Stroke};

/// CRT scanlines: 1px black line every 3px, alpha 0.10. Draw last, over everything.
pub fn scanlines(p: &Painter, rect: Rect) {
    let stroke = Stroke::new(1.0, Color32::from_black_alpha(26));
    let mut y = rect.top();
    while y < rect.bottom() {
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], stroke);
        y += 3.0;
    }
}

/// A faint 60px band that slowly descends the window (40 px/s) and loops.
/// This is the "idle motion" of the whole shell — keep it the only fast-ish motion on screen.
pub fn scan_band(p: &Painter, rect: Rect, t: f64, color: Color32) {
    const LINES: usize = 30;
    const STEP: f32 = 2.0;
    let period = rect.height() + LINES as f32 * STEP;
    // wrap in f64 first: casting `t * 40` to f32 loses sub-pixel precision after ~4.8 days of uptime
    let phase = ((t * 40.0) % period as f64) as f32;
    let head = rect.top() - LINES as f32 * STEP + phase;
    for i in 0..LINES {
        let y = head + i as f32 * STEP;
        if y < rect.top() || y > rect.bottom() {
            continue;
        }
        // triangular alpha distribution, peak 0.045 in the middle
        let f = 1.0 - ((i as f32 - LINES as f32 / 2.0).abs() / (LINES as f32 / 2.0));
        let a = 0.045 * f;
        p.line_segment(
            [pos2(rect.left(), y), pos2(rect.right(), y)],
            Stroke::new(STEP, color.gamma_multiply(a)),
        );
    }
}
