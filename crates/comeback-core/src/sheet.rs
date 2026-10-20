use image::imageops::{self, FilterType};
use image::{Rgb, RgbImage};

pub const SHEET_MAX_EDGE: u32 = 1600;
pub const MAX_PIXELS: u64 = 1_560_000;
const GUTTER: f64 = 6.0;
const ROW_TARGET_H: f64 = 400.0;
const BG: Rgb<u8> = Rgb([17, 17, 17]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Lays out frames with the given width/height ratios in time order.
/// Returns canvas width, canvas height and one cell per frame.
pub fn layout(aspects: &[f64]) -> (u32, u32, Vec<Cell>) {
    let max_w = SHEET_MAX_EDGE as f64;
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut row: Vec<usize> = Vec::new();
    let mut row_w = 0.0;
    for (i, a) in aspects.iter().enumerate() {
        let add = a * ROW_TARGET_H + if row.is_empty() { 0.0 } else { GUTTER };
        if !row.is_empty() && row_w + add > max_w {
            rows.push(std::mem::take(&mut row));
            row_w = 0.0;
        }
        row_w += a * ROW_TARGET_H + if row.is_empty() { 0.0 } else { GUTTER };
        row.push(i);
    }
    if !row.is_empty() {
        rows.push(row);
    }

    // (x, y, w, h) in unscaled float coordinates
    let mut rects = vec![(0.0, 0.0, 0.0, 0.0); aspects.len()];
    let mut y = 0.0;
    let mut canvas_w: f64 = 0.0;
    for (r, idxs) in rows.iter().enumerate() {
        let sum: f64 = idxs.iter().map(|&i| aspects[i]).sum();
        let gutters = GUTTER * (idxs.len() - 1) as f64;
        let mut h = (max_w - gutters) / sum;
        if r + 1 == rows.len() {
            h = h.min(ROW_TARGET_H);
        }
        let mut x = 0.0;
        for &i in idxs {
            let w = aspects[i] * h;
            rects[i] = (x, y, w, h);
            x += w + GUTTER;
        }
        canvas_w = canvas_w.max(x - GUTTER);
        y += h + GUTTER;
    }
    let canvas_h = y - GUTTER;

    let f = 1f64
        .min(SHEET_MAX_EDGE as f64 / canvas_h)
        .min((MAX_PIXELS as f64 / (canvas_w * canvas_h)).sqrt());
    let cells = rects
        .iter()
        .map(|&(x, y, w, h)| Cell {
            x: (x * f).floor() as u32,
            y: (y * f).floor() as u32,
            w: ((w * f).floor() as u32).max(1),
            h: ((h * f).floor() as u32).max(1),
        })
        .collect();
    ((canvas_w * f).floor() as u32, (canvas_h * f).floor() as u32, cells)
}

pub fn render_sheet(frames: &[RgbImage]) -> RgbImage {
    let aspects: Vec<f64> = frames.iter().map(|f| f.width() as f64 / f.height() as f64).collect();
    let (w, h, cells) = layout(&aspects);
    let mut canvas = RgbImage::from_pixel(w, h, BG);
    for (frame, c) in frames.iter().zip(cells) {
        let scaled = imageops::resize(frame, c.w, c.h, FilterType::Triangle);
        imageops::replace(&mut canvas, &scaled, c.x as i64, c.y as i64);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(aspects: &[f64]) -> (u32, u32, Vec<Cell>) {
        let (w, h, cells) = layout(aspects);
        assert_eq!(cells.len(), aspects.len());
        assert!(w <= SHEET_MAX_EDGE && h <= SHEET_MAX_EDGE, "{w}x{h}");
        assert!(w as u64 * h as u64 <= MAX_PIXELS, "{w}x{h}");
        for c in &cells {
            assert!(c.x + c.w <= w && c.y + c.h <= h, "{c:?} outside {w}x{h}");
        }
        for pair in cells.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            assert!(b.y > a.y || (b.y == a.y && b.x >= a.x + a.w), "time order: {a:?} then {b:?}");
        }
        (w, h, cells)
    }

    #[test]
    fn six_wide_frames_fit_the_caps() {
        let (_, _, cells) = check(&[16.0 / 9.0; 6]);
        assert_eq!(cells[0].y, cells[1].y, "two per row");
        assert!(cells[2].y > cells[1].y);
    }

    #[test]
    fn one_frame_is_not_blown_up() {
        let (w, h, _) = check(&[16.0 / 9.0]);
        assert_eq!(h, 400);
        assert_eq!(w, 711);
    }

    #[test]
    fn twelve_mixed_frames_fit() {
        check(&[1.78, 1.78, 0.46, 1.33, 1.78, 2.4, 1.0, 1.78, 0.46, 1.78, 1.78, 1.33]);
    }

    #[test]
    fn render_matches_layout() {
        let frames = vec![RgbImage::from_pixel(160, 90, Rgb([200, 0, 0])); 3];
        let (w, h, _) = layout(&[16.0 / 9.0; 3]);
        let img = render_sheet(&frames);
        assert_eq!(img.dimensions(), (w, h));
    }
}
