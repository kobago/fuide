//! Minimal sfnt (TTF / OTF / TTC) line-metrics reader, used to align fallback fonts (e.g. a CJK
//! system font) to the primary face's baseline.
//!
//! egui places a glyph from a fallback face at
//! `face.ascent + 0.5 * (primary.row_height - face.row_height)` (epaint `text_layout.rs`), so faces
//! with a very different `lineGap` (Hiragino: 0.5 em) end up on a different baseline than the
//! primary face. [`fallback_y_offset_factor`] returns the `FontTweak::y_offset_factor` that
//! cancels the difference. Metric selection mirrors skrifa: OS/2 typo metrics when
//! `USE_TYPO_METRICS` is set, else `hhea`, else OS/2 win metrics.

/// Line metrics in em units (descent is negative, as in the tables).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub line_gap: f32,
}

impl LineMetrics {
    pub fn row_height(&self) -> f32 {
        self.ascent - self.descent + self.line_gap
    }
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]))
}
fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}
fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read the line metrics of face `index` (0 for plain .ttf/.otf) from raw font bytes.
pub fn line_metrics(data: &[u8], index: u32) -> Option<LineMetrics> {
    let mut off = 0usize;
    if data.get(0..4) == Some(b"ttcf") {
        let n = u32_at(data, 8)?;
        if index >= n {
            return None;
        }
        off = u32_at(data, 12 + 4 * index as usize)? as usize;
    }
    let num_tables = u16_at(data, off + 4)? as usize;
    let table = |tag: &[u8; 4]| -> Option<(usize, usize)> {
        (0..num_tables).find_map(|i| {
            let rec = off + 12 + i * 16;
            (data.get(rec..rec + 4)? == tag)
                .then(|| {
                    Some((
                        u32_at(data, rec + 8)? as usize,
                        u32_at(data, rec + 12)? as usize,
                    ))
                })
                .flatten()
        })
    };
    let (head, _) = table(b"head")?;
    let upem = u16_at(data, head + 18)? as f32;
    if upem <= 0.0 {
        return None;
    }
    let hhea = table(b"hhea").and_then(|(o, _)| {
        Some(LineMetrics {
            ascent: i16_at(data, o + 4)? as f32 / upem,
            descent: i16_at(data, o + 6)? as f32 / upem,
            line_gap: i16_at(data, o + 8)? as f32 / upem,
        })
    });
    let os2 = table(b"OS/2").and_then(|(o, len)| {
        if len < 78 {
            return None;
        }
        let use_typo = u16_at(data, o + 62)? & (1 << 7) != 0;
        let typo = LineMetrics {
            ascent: i16_at(data, o + 68)? as f32 / upem,
            descent: i16_at(data, o + 70)? as f32 / upem,
            line_gap: i16_at(data, o + 72)? as f32 / upem,
        };
        let win = LineMetrics {
            ascent: u16_at(data, o + 74)? as f32 / upem,
            descent: -(u16_at(data, o + 76)? as f32) / upem,
            line_gap: 0.0,
        };
        Some((use_typo, typo, win))
    });
    match (os2, hhea) {
        (Some((true, typo, _)), _) => Some(typo),
        (_, Some(h)) if h.ascent != 0.0 || h.descent != 0.0 => Some(h),
        (Some((_, _, win)), _) => Some(win),
        (None, h) => h,
    }
}

/// `FontTweak::y_offset_factor` that puts glyphs of `face` on the baseline of `primary` when
/// egui mixes them in one row.
pub fn fallback_y_offset_factor(primary: LineMetrics, face: LineMetrics) -> f32 {
    let egui_baseline = face.ascent + 0.5 * (primary.row_height() - face.row_height());
    primary.ascent - egui_baseline
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHARE_TECH: &[u8] = include_bytes!("../../../assets/fonts/ShareTechMono.ttf");
    const ORBITRON: &[u8] = include_bytes!("../../../assets/fonts/Orbitron.ttf");

    #[test]
    fn reads_bundled_fonts() {
        let s = line_metrics(SHARE_TECH, 0).unwrap();
        assert!((s.ascent - 0.885).abs() < 1e-3 && (s.descent + 0.242).abs() < 1e-3);
        assert_eq!(s.line_gap, 0.0);
        let o = line_metrics(ORBITRON, 0).unwrap(); // USE_TYPO_METRICS set
        assert!((o.ascent - 1.011).abs() < 1e-3 && (o.descent + 0.243).abs() < 1e-3);
    }

    #[test]
    fn same_face_needs_no_offset() {
        let s = line_metrics(SHARE_TECH, 0).unwrap();
        assert!(fallback_y_offset_factor(s, s).abs() < 1e-6);
    }

    #[test]
    fn hiragino_like_face_is_shifted_down() {
        let s = line_metrics(SHARE_TECH, 0).unwrap();
        let hira = LineMetrics {
            ascent: 0.88,
            descent: -0.12,
            line_gap: 0.5,
        };
        let f = fallback_y_offset_factor(s, hira);
        assert!((f - 0.1915).abs() < 1e-3, "got {f}");
    }

    #[test]
    fn rejects_garbage() {
        assert!(line_metrics(b"not a font", 0).is_none());
        assert!(line_metrics(b"ttcf\0\0\0\x01\0\0\0\x01", 5).is_none()); // ttc index out of range
    }
}
