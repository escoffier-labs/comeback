//! A synthetic 20 s capture: a click on "New tab" at 0:03, a panel appearing at 0:05,
//! a red circle drawn from 0:08 to 0:09.2, and the background going white at 0:12.

use std::path::Path;

use anyhow::Result;
use comeback_core::compile::{compile, CompileInput};
use comeback_core::grid::LumaGrid;
use comeback_core::keyframes::{Candidate, KeyframeConfig};
use comeback_core::model::{Event, EventKind, Manifest, Mark, Target, TargetKind, Word};
use image::{Rgb, RgbImage};

const DURATION_MS: u64 = 20_000;
const MARK_CENTER: (f64, f64) = (0.75, 0.3);
const MARK_RADIUS: f64 = 0.08;

fn rect(img: &mut RgbImage, x0: f64, y0: f64, x1: f64, y1: f64, c: Rgb<u8>) {
    let (w, h) = (img.width() as f64, img.height() as f64);
    for y in (y0 * h) as u32..((y1 * h) as u32).min(img.height()) {
        for x in (x0 * w) as u32..((x1 * w) as u32).min(img.width()) {
            img.put_pixel(x, y, c);
        }
    }
}

fn frame(t: u64, w: u32, h: u32) -> RgbImage {
    let bg = if t >= 12_000 { Rgb([230, 230, 230]) } else { Rgb([40, 44, 52]) };
    let mut img = RgbImage::from_pixel(w, h, bg);
    rect(&mut img, 0.05, 0.05, 0.95, 0.12, Rgb([70, 74, 82]));
    rect(&mut img, 0.45, 0.88, 0.57, 0.97, Rgb([40, 110, 220]));
    if t >= 5_000 {
        rect(&mut img, 0.55, 0.15, 0.95, 0.85, Rgb([200, 200, 210]));
    }
    if (8_000..12_200).contains(&t) {
        let (cx, cy) = (MARK_CENTER.0 * w as f64, MARK_CENTER.1 * h as f64);
        let r = MARK_RADIUS * h as f64;
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                if (d - r).abs() <= 2.0 {
                    img.put_pixel(x, y, Rgb([230, 30, 30]));
                }
            }
        }
    }
    img
}

fn words() -> Vec<Word> {
    [
        ("Clicking", 2_000, 2_400),
        ("new", 2_450, 2_600),
        ("tab", 2_650, 2_900),
        ("does", 3_100, 3_300),
        ("nothing.", 3_350, 3_800),
        ("Then", 5_600, 5_800),
        ("this", 5_850, 6_000),
        ("panel", 6_050, 6_400),
        ("shows", 6_450, 6_700),
        ("up.", 6_750, 7_000),
        ("This", 8_000, 8_200),
        ("label", 8_250, 8_500),
        ("is", 8_550, 8_650),
        ("wrong.", 8_700, 9_100),
        ("And", 12_400, 12_600),
        ("now", 12_650, 12_850),
        ("everything", 12_900, 13_400),
        ("goes", 13_450, 13_650),
        ("white.", 13_700, 14_100),
    ]
    .into_iter()
    .map(|(w, s, e)| Word { w: w.into(), start_ms: s, end_ms: e })
    .collect()
}

fn mark() -> Mark {
    let stroke = (0..=24)
        .map(|i| {
            let a = i as f64 / 24.0 * std::f64::consts::TAU;
            [
                MARK_CENTER.0 + MARK_RADIUS * 9.0 / 16.0 * a.cos(),
                MARK_CENTER.1 + MARK_RADIUS * a.sin(),
                8_000.0 + 1_200.0 * i as f64 / 24.0,
            ]
        })
        .collect();
    Mark {
        id: 1,
        at_ms: 8_000,
        end_ms: 9_200,
        bbox: [0.70, 0.20, 0.80, 0.40],
        strokes: vec![stroke],
        keyframe: None,
    }
}

pub fn run(out: &Path, id: String) -> Result<Manifest> {
    let candidates = (0..=DURATION_MS / 250)
        .map(|i| Candidate { at_ms: i * 250, grid: LumaGrid::from_image(&frame(i * 250, 480, 270)) })
        .collect();
    let input = CompileInput {
        id,
        started_at: "2026-10-19T00:00:00Z".into(),
        backend: "demo".into(),
        target: Target {
            kind: TargetKind::Screen,
            rect: [0, 0, 1280, 720],
            scale: 1.0,
            app: Some("demo".into()),
            title: Some("Settings".into()),
        },
        duration_ms: DURATION_MS,
        events: vec![Event {
            at_ms: 3_000,
            kind: EventKind::Click,
            x: 0.51,
            y: 0.93,
            to: None,
            button: Some("left".into()),
            label: Some("New tab".into()),
        }],
        marks: vec![mark()],
        words: words(),
        transcript_model: Some("demo".into()),
        candidates,
        page: vec![],
        notes: vec![],
    };
    compile(input, &KeyframeConfig::default(), |t| Ok(frame(t, 1280, 720)), out)
}
