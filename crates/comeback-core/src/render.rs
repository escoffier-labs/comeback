use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{Rgb, RgbImage};

use crate::time::fmt_ms;

static FONT: &[u8] = include_bytes!("../assets/DejaVuSans-Bold.ttf");

pub const WHITE: Rgb<u8> = Rgb([255, 255, 255]);
pub const BLACK: Rgb<u8> = Rgb([0, 0, 0]);
const PILL: Rgb<u8> = Rgb([24, 24, 24]);
const RIPPLE: Rgb<u8> = Rgb([255, 200, 0]);

fn font() -> FontRef<'static> {
    FontRef::try_from_slice(FONT).expect("bundled font parses")
}

fn blend(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>, a: f32) {
    if x < 0 || y < 0 || x >= img.width() as i32 || y >= img.height() as i32 {
        return;
    }
    let p = img.get_pixel_mut(x as u32, y as u32);
    for i in 0..3 {
        p[i] = (p[i] as f32 * (1.0 - a) + c[i] as f32 * a).round() as u8;
    }
}

fn fill_rect(img: &mut RgbImage, x: f32, y: f32, w: f32, h: f32, c: Rgb<u8>) {
    for py in y as i32..(y + h) as i32 {
        for px in x as i32..(x + w) as i32 {
            blend(img, px, py, c, 1.0);
        }
    }
}

pub fn text_width(text: &str, px: f32) -> f32 {
    let f = font();
    let sf = f.as_scaled(PxScale::from(px));
    text.chars().map(|c| sf.h_advance(f.glyph_id(c))).sum()
}

/// Draws `text` with its top-left at (x, y).
pub fn draw_text(img: &mut RgbImage, x: f32, y: f32, px: f32, color: Rgb<u8>, text: &str) {
    let f = font();
    let scale = PxScale::from(px);
    let sf = f.as_scaled(scale);
    let baseline = y + sf.ascent();
    let mut caret = x;
    for c in text.chars() {
        let id = f.glyph_id(c);
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        caret += sf.h_advance(id);
        if let Some(o) = f.outline_glyph(glyph) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                blend(img, b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, color, cov);
            });
        }
    }
}

/// White `#N` keycap and dark `M:SS` pill in the top-left corner.
pub fn draw_badge(img: &mut RgbImage, n: u32, at_ms: u64) {
    let s = (img.width().min(img.height()) as f32 / 30.0).clamp(14.0, 48.0);
    let pad = (s * 0.35).round();
    let m = (s * 0.6).round();
    let num = format!("#{n}");
    let time = fmt_ms(at_ms);
    let w1 = text_width(&num, s) + 2.0 * pad;
    let w2 = text_width(&time, s) + 2.0 * pad;
    let h = s + 2.0 * pad;
    fill_rect(img, m, m, w1, h, WHITE);
    draw_text(img, m + pad, m + pad, s, BLACK, &num);
    fill_rect(img, m + w1, m, w2, h, PILL);
    draw_text(img, m + w1 + pad, m + pad, s, WHITE, &time);
}

/// Two yellow rings around a click point given as fractions of the image.
pub fn draw_ripple(img: &mut RgbImage, x: f64, y: f64) {
    let r = (img.width() as f32 / 60.0).clamp(10.0, 40.0);
    let cx = (x * img.width() as f64) as f32;
    let cy = (y * img.height() as f64) as f32;
    for (radius, alpha) in [(r, 0.9f32), (r * 1.6, 0.5)] {
        let reach = (radius + 2.0) as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let d = ((dx * dx + dy * dy) as f32).sqrt();
                if (d - radius).abs() <= 1.5 {
                    blend(img, cx as i32 + dx, cy as i32 + dy, RIPPLE, alpha);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_paints_a_white_keycap_top_left() {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        draw_badge(&mut img, 3, 12_000);
        // font px = clamp(360 / 30, 14, 48) = 14, margin = round(14 * 0.6) = 8
        assert_eq!(*img.get_pixel(9, 9), WHITE);
        assert_eq!(*img.get_pixel(600, 300), Rgb([0, 0, 0]));
    }

    #[test]
    fn ripple_rings_the_click_point() {
        let mut img = RgbImage::from_pixel(640, 360, Rgb([0, 0, 0]));
        draw_ripple(&mut img, 0.5, 0.5);
        // radius = clamp(640 / 60, 10, 40), about 10.7
        assert_ne!(*img.get_pixel(330, 180), Rgb([0, 0, 0]));
        assert_eq!(*img.get_pixel(320, 180), Rgb([0, 0, 0]), "the centre stays clear");
    }

    #[test]
    fn text_has_width() {
        assert!(text_width("#12", 20.0) > 20.0);
    }
}
