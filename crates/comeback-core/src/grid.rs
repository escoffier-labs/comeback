use image::RgbImage;

pub const GRID_W: usize = 32;
pub const GRID_H: usize = 18;
/// A cell counts as changed when its mean luma moves by more than this.
/// Thin ink strokes stay under it, so ink alone does not read as a new screen.
const CELL_DELTA: u8 = 12;

/// Mean luma per cell over a 32x18 grid.
#[derive(Debug, Clone, PartialEq)]
pub struct LumaGrid {
    cells: Vec<u8>,
}

impl LumaGrid {
    pub fn from_image(img: &RgbImage) -> Self {
        let (w, h) = img.dimensions();
        let mut sums = vec![0u64; GRID_W * GRID_H];
        let mut counts = vec![0u64; GRID_W * GRID_H];
        for (x, y, p) in img.enumerate_pixels() {
            let cx = (x as usize * GRID_W) / w as usize;
            let cy = (y as usize * GRID_H) / h as usize;
            let l = (299 * p[0] as u64 + 587 * p[1] as u64 + 114 * p[2] as u64) / 1000;
            sums[cy * GRID_W + cx] += l;
            counts[cy * GRID_W + cx] += 1;
        }
        let cells = sums
            .iter()
            .zip(&counts)
            .map(|(s, c)| (s / (*c).max(1)) as u8)
            .collect();
        Self { cells }
    }

    /// Fraction of all cells that changed.
    pub fn diff(&self, other: &Self) -> f64 {
        self.diff_in(other, [0.0, 0.0, 1.0, 1.0])
    }

    /// Fraction of changed cells inside `bbox` (x0, y0, x1, y1 as fractions).
    pub fn diff_in(&self, other: &Self, bbox: [f64; 4]) -> f64 {
        let span = |lo: f64, hi: f64, n: usize| {
            let a = ((lo.clamp(0.0, 1.0) * n as f64).floor() as usize).min(n - 1);
            let b = ((hi.clamp(0.0, 1.0) * n as f64).ceil() as usize).clamp(a + 1, n);
            (a, b)
        };
        let (x0, x1) = span(bbox[0], bbox[2], GRID_W);
        let (y0, y1) = span(bbox[1], bbox[3], GRID_H);
        let mut changed = 0usize;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * GRID_W + x;
                if self.cells[i].abs_diff(other.cells[i]) > CELL_DELTA {
                    changed += 1;
                }
            }
        }
        changed as f64 / ((x1 - x0) * (y1 - y0)) as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn solid(v: u8) -> LumaGrid {
        LumaGrid::from_image(&RgbImage::from_pixel(64, 36, Rgb([v, v, v])))
    }

    #[test]
    fn identical_images_do_not_differ() {
        assert_eq!(solid(10).diff(&solid(10)), 0.0);
        assert_eq!(
            solid(10).diff(&solid(20)),
            0.0,
            "delta 10 is under the cell threshold"
        );
    }

    #[test]
    fn black_and_white_differ_everywhere() {
        assert_eq!(solid(0).diff(&solid(255)), 1.0);
    }

    #[test]
    fn half_changed_image_is_half_different() {
        let mut img = RgbImage::from_pixel(64, 36, Rgb([0, 0, 0]));
        for y in 0..36 {
            for x in 32..64 {
                img.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
        let half = LumaGrid::from_image(&img);
        assert_eq!(solid(0).diff(&half), 0.5);
        assert_eq!(solid(0).diff_in(&half, [0.0, 0.0, 0.5, 1.0]), 0.0);
        assert_eq!(solid(0).diff_in(&half, [0.5, 0.0, 1.0, 1.0]), 1.0);
    }
}
