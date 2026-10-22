//! Capture target geometry: `--area` parsing and fitting a rectangle onto the screen.

use anyhow::{Result, bail};

/// Smallest capture edge in pixels. Smaller areas make unreadable frames.
pub const MIN_EDGE: i32 = 16;

/// Parses `X,Y,W,H` (spaces allowed, X and Y may be negative on multi-monitor desktops).
pub fn parse_area(s: &str) -> Result<[i32; 4]> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    let nums: Option<Vec<i32>> = parts.iter().map(|p| p.parse().ok()).collect();
    let [x, y, w, h] = match nums.as_deref() {
        Some(&[x, y, w, h]) => [x, y, w, h],
        _ => bail!("--area wants X,Y,W,H as four whole numbers, got {s:?}"),
    };
    if w <= 0 || h <= 0 {
        bail!("--area width and height must be positive, got {w}x{h}");
    }
    Ok([x, y, w, h])
}

/// Checks `area` lies inside `screen` and rounds its size down to even numbers,
/// because the H.264 encoder rejects odd frame sizes.
pub fn fit_area(area: [i32; 4], screen: [i32; 4]) -> Result<[i32; 4]> {
    let [x, y, w, h] = area;
    let [sx, sy, sw, sh] = screen;
    let (w, h) = (w & !1, h & !1);
    if w < MIN_EDGE || h < MIN_EDGE {
        bail!("area must be at least {MIN_EDGE}x{MIN_EDGE} pixels, got {w}x{h}");
    }
    let inside = x >= sx
        && y >= sy
        && i64::from(x) + i64::from(w) <= i64::from(sx) + i64::from(sw)
        && i64::from(y) + i64::from(h) <= i64::from(sy) + i64::from(sh);
    if !inside {
        bail!("area {x},{y},{w},{h} is outside the screen ({sw}x{sh} at {sx},{sy})");
    }
    Ok([x, y, w, h])
}

/// The part of `rect` that is on the screen (windows can hang off the edge).
pub fn clamp_to_screen(rect: [i32; 4], screen: [i32; 4]) -> Result<[i32; 4]> {
    let x0 = rect[0].max(screen[0]);
    let y0 = rect[1].max(screen[1]);
    let x1 = (rect[0] + rect[2]).min(screen[0] + screen[2]);
    let y1 = (rect[1] + rect[3]).min(screen[1] + screen[3]);
    if x1 <= x0 || y1 <= y0 {
        bail!("the window is entirely off the screen");
    }
    Ok([x0, y0, x1 - x0, y1 - y0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_spaced_areas() {
        assert_eq!(parse_area("0,0,640,360").unwrap(), [0, 0, 640, 360]);
        assert_eq!(
            parse_area(" 10, 20 ,300, 200 ").unwrap(),
            [10, 20, 300, 200]
        );
        assert_eq!(parse_area("-1920,0,640,360").unwrap(), [-1920, 0, 640, 360]);
    }

    #[test]
    fn rejects_malformed_areas() {
        for bad in [
            "",
            "1,2,3",
            "1,2,3,4,5",
            "a,b,c,d",
            "0,0,0,10",
            "0,0,10,0",
            "0,0,-5,10",
        ] {
            assert!(parse_area(bad).is_err(), "{bad:?} should fail");
        }
        let msg = parse_area("1,2,3").unwrap_err().to_string();
        assert!(msg.contains("X,Y,W,H"), "{msg}");
    }

    #[test]
    fn fits_inside_the_screen_and_rounds_to_even() {
        let screen = [0, 0, 1280, 720];
        assert_eq!(
            fit_area([10, 10, 641, 361], screen).unwrap(),
            [10, 10, 640, 360]
        );
        assert_eq!(
            fit_area([0, 0, 1280, 720], screen).unwrap(),
            [0, 0, 1280, 720]
        );
    }

    #[test]
    fn rejects_areas_outside_or_too_small() {
        let screen = [0, 0, 1280, 720];
        let outside = fit_area([1000, 0, 400, 300], screen)
            .unwrap_err()
            .to_string();
        assert!(outside.contains("outside the screen"), "{outside}");
        assert!(fit_area([-1, 0, 100, 100], screen).is_err());
        let tiny = fit_area([0, 0, 8, 100], screen).unwrap_err().to_string();
        assert!(tiny.contains("at least 16"), "{tiny}");
    }

    #[test]
    fn windows_are_clamped_to_the_screen() {
        let screen = [0, 0, 1280, 720];
        assert_eq!(
            clamp_to_screen([-50, 10, 300, 200], screen).unwrap(),
            [0, 10, 250, 200]
        );
        assert_eq!(
            clamp_to_screen([1200, 600, 300, 300], screen).unwrap(),
            [1200, 600, 80, 120]
        );
        assert!(clamp_to_screen([2000, 0, 100, 100], screen).is_err());
    }

    #[test]
    fn screen_origin_may_be_offset() {
        let screen = [-1920, 0, 3840, 1080];
        assert_eq!(
            fit_area([-1920, 0, 200, 100], screen).unwrap(),
            [-1920, 0, 200, 100]
        );
    }
}
