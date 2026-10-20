# comeback plan 1: core crate, demo and inspect

> For agentic workers: execute this plan task by task, in order, checking off each `- [ ]` as you go. Do not skip the failing-test runs. The plan is the source of truth. Chat history is not. If `.brigade/` exists in the repo, wrap every `cargo` check in `brigade work verify run --target . --command "<cmd>" --capture brigade-work`. End every commit message with the co-author trailer for the tool doing the work (for Claude: `Co-Authored-By: Claude <noreply@anthropic.com>`).

**Goal:** a platform-free `comeback-core` crate that turns a timeline (events, ink marks, words, low-res candidate frames) into a finished bundle, plus a `comeback` binary with `demo` and `inspect`.

**Architecture:** Cargo workspace with two crates. `comeback-core` holds pure logic: manifest types, a luma grid for scene diffs, keyframe selection, badge and ripple drawing, contact sheet layout, summary text, the compile step and bundle loading. It takes frames through a closure, so it never touches capture hardware. `comeback` is the CLI. Plan 1 has only `demo` (synthetic capture) and `inspect`.

**Tech:** Rust 2024, `serde`/`serde_json`, `image` 0.25 (PNG only), `ab_glyph` with bundled DejaVu Sans Bold, `clap` 4, `anyhow`, `tempfile` for tests.

**Spec:** `docs/specs/2026-10-19-comeback-v1.md`. This plan covers its "Keyframe selection", "Bundle format", and the `inspect` and `demo` CLI rows. Later plans cover the rest (listed at the end).

## File map

| File | Responsibility |
|---|---|
| `Cargo.toml` | Workspace, shared dependency versions |
| `LICENSE` | MIT, Solomon Neas |
| `.gitignore` | `target/`, local handoff inbox (exists before Task 1) |
| `.github/workflows/ci.yml` | fmt, clippy, test |
| `crates/comeback-core/Cargo.toml` | Core crate manifest |
| `crates/comeback-core/assets/DejaVuSans-Bold.ttf` | Badge font, embedded with `include_bytes!` |
| `crates/comeback-core/assets/DejaVu-LICENSE` | Font license |
| `crates/comeback-core/src/lib.rs` | Module list |
| `crates/comeback-core/src/model.rs` | `comeback.json` types |
| `crates/comeback-core/src/time.rs` | `fmt_ms` (M:SS) |
| `crates/comeback-core/src/grid.rs` | `LumaGrid` and diffs |
| `crates/comeback-core/src/keyframes.rs` | Transitions, mark grouping, selection, word ownership |
| `crates/comeback-core/src/render.rs` | Text, badge, ripple drawing |
| `crates/comeback-core/src/sheet.rs` | Contact sheet layout and render |
| `crates/comeback-core/src/summary.rs` | Paste text and `summary.md` |
| `crates/comeback-core/src/compile.rs` | Timeline plus frames to bundle directory |
| `crates/comeback-core/src/bundle.rs` | Load a bundle, resolve `latest` |
| `crates/comeback/Cargo.toml` | Binary crate manifest |
| `crates/comeback/src/main.rs` | clap CLI, captures dir |
| `crates/comeback/src/demo.rs` | Synthetic 20 s capture |
| `crates/comeback/tests/cli.rs` | End-to-end CLI tests |

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `LICENSE`, `crates/comeback-core/Cargo.toml`, `crates/comeback-core/src/lib.rs`, `crates/comeback/Cargo.toml`, `crates/comeback/src/main.rs`, `crates/comeback-core/assets/*`

- [x] Create `Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/comeback-core", "crates/comeback"]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"

[workspace.dependencies]
anyhow = "1.0.104"
ab_glyph = "0.2.32"
clap = { version = "4.6.7", features = ["derive"] }
image = { version = "0.25.10", default-features = false, features = ["png"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = "1.0.154"
tempfile = "3.27.0"
```

- [x] Create `crates/comeback-core/Cargo.toml`:

```toml
[package]
name = "comeback-core"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
ab_glyph.workspace = true
anyhow.workspace = true
image.workspace = true
serde.workspace = true
serde_json.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [x] Create `crates/comeback/Cargo.toml`:

```toml
[package]
name = "comeback"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
anyhow.workspace = true
clap.workspace = true
comeback-core = { path = "../comeback-core" }
image.workspace = true
serde_json.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [x] Create `crates/comeback-core/src/lib.rs` (empty for now):

```rust
//! Platform-free core of comeback: timeline in, bundle out.
```

- [x] Create `crates/comeback/src/main.rs`:

```rust
fn main() {}
```

- [x] `.gitignore` already exists with `target/` and `.claude/memory-handoffs/`. Leave it as is.
- [x] Create `LICENSE`: the standard MIT text with the line `Copyright (c) 2026 Solomon Neas`.
- [x] Copy the font and its license:

```sh
mkdir -p crates/comeback-core/assets
cp /usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf crates/comeback-core/assets/
cp /usr/share/doc/fonts-dejavu-core/copyright crates/comeback-core/assets/DejaVu-LICENSE
```

- [x] Run `cargo check --workspace`. Expect `Finished` with no errors.
- [x] Commit: `git add -A && git commit -m "build: scaffold comeback workspace"`

---

### Task 2: Manifest types

**Files:**
- Create: `crates/comeback-core/src/model.rs`
- Modify: `crates/comeback-core/src/lib.rs`

- [x] Add `pub mod model;` to `lib.rs`, then create `model.rs` holding only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Manifest {
        Manifest {
            schema: SCHEMA,
            id: "comeback-1".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            duration_ms: 33_000,
            backend: "x11".into(),
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 2560, 1440],
                scale: 1.0,
                app: Some("firefox".into()),
                title: Some("Settings".into()),
            },
            keyframes: vec![Keyframe {
                n: 1,
                at_ms: 4_000,
                reason: Reason::Click,
                file: "frames/frame-01.png".into(),
                said: "hi".into(),
                action: Some("clicked".into()),
            }],
            marks: vec![],
            events: vec![Event {
                at_ms: 4_000,
                kind: EventKind::Click,
                x: 0.5,
                y: 0.5,
                to: None,
                button: Some("left".into()),
                label: None,
            }],
            page: vec![],
            transcript: Some(Transcript {
                model: "small.en".into(),
                words: vec![Word { w: "hi".into(), start_ms: 4_000, end_ms: 4_200 }],
            }),
            notes: vec![],
        }
    }

    #[test]
    fn round_trips_with_camel_case_keys() {
        let m = sample();
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"durationMs\":33000"));
        assert!(json.contains("\"startMs\":4000"));
        assert!(json.contains("\"reason\":\"click\""));
        assert!(!json.contains("\"notes\""), "empty notes are omitted");
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
```

- [x] Run `cargo test -p comeback-core model`. Expect a compile FAIL: `cannot find type Manifest`.
- [x] Put this above the test module in `model.rs`:

```rust
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub backend: String,
    pub target: Target,
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub page: Vec<PageEvent>,
    pub transcript: Option<Transcript>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Screen,
    Window,
    Area,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub kind: TargetKind,
    /// x, y, width, height in physical pixels, global coordinates.
    pub rect: [i32; 4],
    pub scale: f64,
    pub app: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Mark,
    Click,
    Drag,
    Cue,
    Scene,
    Midpoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    pub n: u32,
    pub at_ms: u64,
    pub reason: Reason,
    pub file: String,
    pub said: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: u32,
    pub at_ms: u64,
    pub end_ms: u64,
    /// x0, y0, x1, y1 as fractions of the target.
    pub bbox: [f64; 4],
    /// Each stroke is a list of [x, y, t_ms]; x and y are fractions.
    pub strokes: Vec<Vec<[f64; 3]>>,
    pub keyframe: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Click,
    Drag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub at_ms: u64,
    pub kind: EventKind,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<[f64; 2]>,
    pub button: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEvent {
    pub at_ms: u64,
    pub kind: String,
    pub url: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub model: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub w: String,
    pub start_ms: u64,
    pub end_ms: u64,
}
```

- [x] Run `cargo test -p comeback-core model`. Expect `1 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): add comeback.json manifest types"`

---

### Task 3: Time formatting

**Files:**
- Create: `crates/comeback-core/src/time.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod time;`)

- [x] Create `time.rs` holding only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_minutes_and_seconds() {
        assert_eq!(fmt_ms(0), "0:00");
        assert_eq!(fmt_ms(4_999), "0:04");
        assert_eq!(fmt_ms(65_000), "1:05");
        assert_eq!(fmt_ms(600_000), "10:00");
    }
}
```

- [x] Run `cargo test -p comeback-core time`. Expect a compile FAIL: `cannot find function fmt_ms`.
- [x] Add above the tests:

```rust
/// Milliseconds as `M:SS`, truncating partial seconds.
pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
```

- [x] Run `cargo test -p comeback-core time`. Expect `1 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): add M:SS time formatting"`

---

### Task 4: Luma grid

**Files:**
- Create: `crates/comeback-core/src/grid.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod grid;`)

- [x] Create `grid.rs` holding only the tests:

```rust
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
        assert_eq!(solid(10).diff(&solid(20)), 0.0, "delta 10 is under the cell threshold");
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
```

- [x] Run `cargo test -p comeback-core grid`. Expect a compile FAIL: `cannot find type LumaGrid`.
- [x] Add above the tests:

```rust
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
```

- [x] Run `cargo test -p comeback-core grid`. Expect `3 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): add luma grid for scene diffs"`

---

### Task 5: Keyframe selection

**Files:**
- Create: `crates/comeback-core/src/keyframes.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod keyframes;`)

These rules come from the spec's "Keyframe selection" section. Decisions the spec left implicit, pinned here:
- A scene transition starts at the first candidate that differs from the last settled frame by `scene_threshold` or more. It settles at the first candidate `j` where `diff(j, j+1)` and `diff(j+1, j+2)` are both under `settle_threshold`. The last candidate counts as settled.
- Mark groups compare the frame at the previous mark's end with the frame at the new mark's start, inside their padded union box.
- A mark group's frame is 100 ms after its last mark ends (ink stays on screen 3 s), clamped to the duration.
- Fill order after marks: clicks and drags, then cues, then scenes, each in time order. A candidate within 500 ms of a chosen keyframe is discarded. Cues must also differ by `cue_novelty` (0.03) or more from every chosen keyframe's frame. Scene changes must differ by that much from every chosen cue keyframe, so a cue and the transition it snapped to never both appear. Candidates turned away only because the budget is full go to a leftover pool for coverage.
- Coverage may add keyframes past the budget, up to `hard_cap` (12), or past it only if marks alone exceed 12.
- When an anchor falls in silence after a finished clause, the keyframe's words start at the next clause.

- [x] Create `keyframes.rs` holding only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventKind;
    use image::{Rgb, RgbImage};

    fn cand(at_ms: u64, v: u8) -> Candidate {
        Candidate {
            at_ms,
            grid: LumaGrid::from_image(&RgbImage::from_pixel(64, 36, Rgb([v, v, v]))),
        }
    }
    fn click(at_ms: u64, label: Option<&str>) -> Event {
        Event {
            at_ms,
            kind: EventKind::Click,
            x: 0.5,
            y: 0.5,
            to: None,
            button: Some("left".into()),
            label: label.map(Into::into),
        }
    }
    fn word(w: &str, start_ms: u64, end_ms: u64) -> Word {
        Word { w: w.into(), start_ms, end_ms }
    }
    fn mark(id: u32, at_ms: u64, end_ms: u64) -> Mark {
        Mark {
            id,
            at_ms,
            end_ms,
            bbox: [0.4, 0.4, 0.6, 0.6],
            strokes: vec![vec![[0.4, 0.4, at_ms as f64], [0.6, 0.6, end_ms as f64]]],
            keyframe: None,
        }
    }
    fn run(duration_ms: u64, events: &[Event], marks: &[Mark], words: &[Word], candidates: &[Candidate]) -> Vec<Selected> {
        select(
            &SelectInput { duration_ms, events, marks, words, candidates },
            &KeyframeConfig::default(),
        )
    }

    #[test]
    fn budget_steps() {
        assert_eq!(budget(40_000), 6);
        assert_eq!(budget(40_001), 7);
        assert_eq!(budget(60_000), 7);
        assert_eq!(budget(60_001), 8);
        assert_eq!(budget(600_000), 12);
    }

    #[test]
    fn finds_a_settled_transition() {
        let mut c: Vec<Candidate> = (0..8).map(|i| cand(i * 250, 0)).collect();
        c.push(cand(2_000, 128));
        c.extend((9..17).map(|i| cand(i * 250, 255)));
        let t = scene_transitions(&c, &KeyframeConfig::default());
        assert_eq!(t, vec![Transition { start_ms: 2_000, settled_ms: 2_250 }]);
    }

    #[test]
    fn labels_clicks() {
        let s = run(10_000, &[click(3_000, Some("New tab"))], &[], &[], &[]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].at_ms, 3_000);
        assert_eq!(s[0].action.as_deref(), Some("clicked \"New tab\""));
        assert_eq!(s[0].point, Some([0.5, 0.5]));
        let s = run(10_000, &[click(3_000, None)], &[], &[], &[]);
        assert_eq!(s[0].action.as_deref(), Some("clicked (x 0.50, y 0.50)"));
    }

    #[test]
    fn groups_marks_on_one_screen() {
        let s = run(10_000, &[], &[mark(1, 1_000, 1_500), mark(2, 2_500, 3_000)], &[], &[]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].reason, Reason::Mark);
        assert_eq!(s[0].at_ms, 3_100);
        assert_eq!(s[0].mark_ids, vec![1, 2]);
        assert_eq!(s[0].action.as_deref(), Some("marked ×2"));
    }

    #[test]
    fn a_click_splits_mark_groups() {
        let s = run(
            10_000,
            &[click(2_000, None)],
            &[mark(1, 1_000, 1_500), mark(2, 2_500, 3_000)],
            &[],
            &[],
        );
        let marks: Vec<_> = s.iter().filter(|k| k.reason == Reason::Mark).collect();
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].mark_ids, vec![1]);
        assert_eq!(marks[1].mark_ids, vec![2]);
        assert_eq!(s.len(), 2, "the click is within 500 ms of mark 1's frame");
    }

    #[test]
    fn cues_snap_to_transitions_and_need_novelty() {
        let c: Vec<Candidate> = (0..=40)
            .map(|i| cand(i * 250, if i * 250 >= 5_500 { 255 } else { 0 }))
            .collect();
        let words = [word("here,", 5_000, 5_300), word("now", 9_000, 9_200)];
        let s = run(10_000, &[], &[], &words, &c);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].reason, Reason::Cue);
        assert_eq!(s[0].at_ms, 5_500);
        assert_eq!(s[0].said, "here, now");
    }

    #[test]
    fn budget_caps_ordinary_events() {
        let events: Vec<Event> = (0..10).map(|k| click(1_000 + 1_500 * k, None)).collect();
        let s = run(20_000, &events, &[], &[], &[]);
        assert_eq!(s.len(), 6);
        assert_eq!(s[0].at_ms, 1_000);
        assert_eq!(s[5].at_ms, 8_500);
    }

    #[test]
    fn coverage_fills_long_gaps_from_leftovers() {
        let events: Vec<Event> = (0..10).map(|k| click(1_000 + 1_500 * k, None)).collect();
        let c: Vec<Candidate> = (0..=120)
            .map(|i| cand(i * 1_000, if i * 1_000 >= 80_000 { 255 } else { 0 }))
            .collect();
        let s = run(120_000, &events, &[], &[], &c);
        assert_eq!(budget(120_000), 10);
        assert_eq!(s.len(), 11);
        assert_eq!(s[10].reason, Reason::Scene);
        assert_eq!(s[10].at_ms, 80_000);
    }

    #[test]
    fn keyframes_own_their_clauses() {
        let words = [
            word("The", 0, 200),
            word("menu", 250, 500),
            word("opens.", 550, 900),
            word("But", 3_000, 3_200),
            word("this", 3_250, 3_400),
            word("label", 3_450, 3_700),
            word("is", 3_750, 3_850),
            word("wrong.", 3_900, 4_300),
        ];
        let s = run(5_000, &[click(600, None), click(3_500, None)], &[], &words, &[]);
        assert_eq!(s[0].said, "The menu opens.");
        assert_eq!(s[1].said, "But this label is wrong.");
        let s = run(5_000, &[click(600, None), click(2_000, None)], &[], &words, &[]);
        assert_eq!(s[1].said, "But this label is wrong.", "an anchor in silence takes the next clause");
    }

    #[test]
    fn nothing_happened_gives_a_midpoint() {
        let s = run(8_000, &[], &[], &[], &[]);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].reason, Reason::Midpoint);
        assert_eq!(s[0].at_ms, 4_000);
    }
}
```

- [x] Run `cargo test -p comeback-core keyframes`. Expect a compile FAIL: `cannot find type Candidate`.
- [x] Add above the tests:

```rust
use crate::grid::LumaGrid;
use crate::model::{Event, EventKind, Mark, Reason, Word};

#[derive(Debug, Clone)]
pub struct KeyframeConfig {
    pub scene_threshold: f64,
    pub settle_threshold: f64,
    pub cue_novelty: f64,
    pub mark_same_screen: f64,
    pub mark_bbox_pad: f64,
    pub mark_group_max_ms: u64,
    pub mark_group_max_strokes: usize,
    pub mark_frame_delay_ms: u64,
    pub cue_before_ms: u64,
    pub cue_after_ms: u64,
    pub cue_fallback_ms: u64,
    pub dedupe_ms: u64,
    pub coverage_ms: u64,
    pub coverage_words: usize,
    pub clause_pause_ms: u64,
    pub hard_cap: usize,
}

impl Default for KeyframeConfig {
    fn default() -> Self {
        Self {
            scene_threshold: 0.08,
            settle_threshold: 0.02,
            cue_novelty: 0.03,
            mark_same_screen: 0.05,
            mark_bbox_pad: 0.08,
            mark_group_max_ms: 20_000,
            mark_group_max_strokes: 12,
            mark_frame_delay_ms: 100,
            cue_before_ms: 300,
            cue_after_ms: 1_200,
            cue_fallback_ms: 150,
            dedupe_ms: 500,
            coverage_ms: 35_000,
            coverage_words: 100,
            clause_pause_ms: 1_500,
            hard_cap: 12,
        }
    }
}

/// A low-resolution frame sampled during recording.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub at_ms: u64,
    pub grid: LumaGrid,
}

pub struct SelectInput<'a> {
    pub duration_ms: u64,
    pub events: &'a [Event],
    pub marks: &'a [Mark],
    pub words: &'a [Word],
    pub candidates: &'a [Candidate],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selected {
    /// Time of the frame to show.
    pub at_ms: u64,
    /// Time the keyframe's words start from.
    pub anchor_ms: u64,
    pub reason: Reason,
    pub action: Option<String>,
    /// Click point to draw a ripple at, as fractions.
    pub point: Option<[f64; 2]>,
    pub mark_ids: Vec<u32>,
    pub said: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub start_ms: u64,
    pub settled_ms: u64,
}

const STRONG_CUES: [&str; 4] = ["now", "here", "look", "watch"];

pub fn budget(duration_ms: u64) -> usize {
    if duration_ms <= 40_000 {
        6
    } else {
        (6 + (duration_ms - 40_000).div_ceil(20_000) as usize).min(12)
    }
}

pub fn scene_transitions(c: &[Candidate], cfg: &KeyframeConfig) -> Vec<Transition> {
    let mut out = Vec::new();
    if c.is_empty() {
        return out;
    }
    let mut settled = 0usize;
    let mut i = 1usize;
    while i < c.len() {
        if c[settled].grid.diff(&c[i].grid) < cfg.scene_threshold {
            i += 1;
            continue;
        }
        let mut j = i;
        while j + 1 < c.len() {
            let a = c[j].grid.diff(&c[j + 1].grid) < cfg.settle_threshold;
            let b = j + 2 >= c.len() || c[j + 1].grid.diff(&c[j + 2].grid) < cfg.settle_threshold;
            if a && b {
                break;
            }
            j += 1;
        }
        out.push(Transition { start_ms: c[i].at_ms, settled_ms: c[j].at_ms });
        settled = j;
        i = j + 1;
    }
    out
}

fn nearest(c: &[Candidate], t: u64) -> Option<&Candidate> {
    c.iter().min_by_key(|k| k.at_ms.abs_diff(t))
}

fn padded_union(a: [f64; 4], b: [f64; 4], pad: f64) -> [f64; 4] {
    [
        a[0].min(b[0]) - pad,
        a[1].min(b[1]) - pad,
        a[2].max(b[2]) + pad,
        a[3].max(b[3]) + pad,
    ]
}

fn group_marks<'m>(input: &SelectInput<'m>, cfg: &KeyframeConfig) -> Vec<Vec<&'m Mark>> {
    let mut sorted: Vec<&Mark> = input.marks.iter().collect();
    sorted.sort_by_key(|m| m.at_ms);
    let mut groups: Vec<Vec<&Mark>> = Vec::new();
    for m in sorted {
        if let Some(g) = groups.last_mut() {
            let first = g[0];
            let last = *g.last().expect("groups are never empty");
            let click_between = input.events.iter().any(|e| e.at_ms > last.end_ms && e.at_ms < m.at_ms);
            let too_long = m.end_ms.saturating_sub(first.at_ms) > cfg.mark_group_max_ms;
            let strokes: usize = g.iter().map(|x| x.strokes.len().max(1)).sum::<usize>() + m.strokes.len().max(1);
            let too_many = strokes > cfg.mark_group_max_strokes;
            let screen_changed = match (nearest(input.candidates, last.end_ms), nearest(input.candidates, m.at_ms)) {
                (Some(a), Some(b)) => {
                    a.grid.diff_in(&b.grid, padded_union(last.bbox, m.bbox, cfg.mark_bbox_pad)) >= cfg.mark_same_screen
                }
                _ => false,
            };
            if !(click_between || too_long || too_many || screen_changed) {
                g.push(m);
                continue;
            }
        }
        groups.push(vec![m]);
    }
    groups
}

fn normalize(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

fn accept(s: &Selected, chosen: &[Selected], input: &SelectInput, cfg: &KeyframeConfig) -> bool {
    if chosen.iter().any(|c| c.at_ms.abs_diff(s.at_ms) < cfg.dedupe_ms) {
        return false;
    }
    // Cues must look new against every keyframe. Scene changes only against cues,
    // so a cue and the transition it snapped to never both appear.
    let against_all = match s.reason {
        Reason::Cue => true,
        Reason::Scene => false,
        _ => return true,
    };
    let Some(mine) = nearest(input.candidates, s.at_ms) else {
        return true;
    };
    chosen.iter().filter(|c| against_all || c.reason == Reason::Cue).all(|c| match nearest(input.candidates, c.at_ms) {
        Some(theirs) => mine.grid.diff(&theirs.grid) >= cfg.cue_novelty,
        None => true,
    })
}

fn gaps(chosen: &[Selected], duration_ms: u64) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut prev = 0u64;
    for s in chosen {
        if s.at_ms > prev {
            out.push((prev, s.at_ms));
        }
        prev = s.at_ms;
    }
    if duration_ms > prev {
        out.push((prev, duration_ms));
    }
    out
}

fn fill_coverage(
    chosen: &mut Vec<Selected>,
    leftovers: &mut Vec<Selected>,
    input: &SelectInput,
    cfg: &KeyframeConfig,
    cap: usize,
) {
    loop {
        if chosen.len() >= cap {
            return;
        }
        chosen.sort_by_key(|s| s.at_ms);
        let mut added = false;
        for (a, b) in gaps(chosen, input.duration_ms) {
            let words = input.words.iter().filter(|w| w.start_ms >= a && w.start_ms < b).count();
            if b - a <= cfg.coverage_ms && words <= cfg.coverage_words {
                continue;
            }
            let mid = a + (b - a) / 2;
            let best = leftovers
                .iter()
                .enumerate()
                .filter(|(_, s)| s.at_ms >= a && s.at_ms < b && accept(s, chosen, input, cfg))
                .min_by_key(|(_, s)| s.at_ms.abs_diff(mid))
                .map(|(i, _)| i);
            if let Some(i) = best {
                chosen.push(leftovers.remove(i));
                added = true;
                break;
            }
        }
        if !added {
            return;
        }
    }
}

fn ends_clause(w: &str) -> bool {
    w.ends_with(['.', ',', '?', '!', ';', ':'])
}

fn assign_words(sel: &mut [Selected], words: &[Word], cfg: &KeyframeConfig) {
    if words.is_empty() || sel.is_empty() {
        return;
    }
    let starts: Vec<bool> = (0..words.len())
        .map(|i| {
            i == 0
                || ends_clause(&words[i - 1].w)
                || words[i].start_ms.saturating_sub(words[i - 1].end_ms) >= cfg.clause_pause_ms
        })
        .collect();
    let mut idx: Vec<usize> = sel
        .iter()
        .map(|s| {
            let mut i = words.iter().rposition(|w| w.start_ms <= s.anchor_ms).unwrap_or(0);
            if words[i].end_ms < s.anchor_ms && i + 1 < words.len() && starts[i + 1] {
                i += 1;
            }
            while i > 0 && !starts[i] {
                i -= 1;
            }
            i
        })
        .collect();
    idx[0] = 0;
    for k in 1..idx.len() {
        idx[k] = idx[k].max(idx[k - 1]);
    }
    for k in 0..sel.len() {
        let end = if k + 1 < sel.len() { idx[k + 1] } else { words.len() };
        sel[k].said = words[idx[k]..end].iter().map(|w| w.w.as_str()).collect::<Vec<_>>().join(" ");
    }
}

pub fn select(input: &SelectInput, cfg: &KeyframeConfig) -> Vec<Selected> {
    let transitions = scene_transitions(input.candidates, cfg);

    let mut chosen: Vec<Selected> = group_marks(input, cfg)
        .into_iter()
        .map(|g| {
            let last = g.last().expect("groups are never empty");
            Selected {
                at_ms: (last.end_ms + cfg.mark_frame_delay_ms).min(input.duration_ms),
                anchor_ms: g[0].at_ms,
                reason: Reason::Mark,
                action: Some(if g.len() == 1 { "marked".into() } else { format!("marked ×{}", g.len()) }),
                point: None,
                mark_ids: g.iter().map(|m| m.id).collect(),
                said: String::new(),
            }
        })
        .collect();
    let cap = cfg.hard_cap.max(chosen.len());
    let budget = budget(input.duration_ms).max(chosen.len());

    let mut pool: Vec<Selected> = Vec::new();
    let mut events: Vec<&Event> = input.events.iter().collect();
    events.sort_by_key(|e| e.at_ms);
    for e in events {
        let (reason, verb) = match e.kind {
            EventKind::Click => (Reason::Click, "clicked"),
            EventKind::Drag => (Reason::Drag, "dragged"),
        };
        let action = match (&e.label, e.kind) {
            (Some(l), _) => format!("{verb} \"{l}\""),
            (None, EventKind::Click) => format!("clicked (x {:.2}, y {:.2})", e.x, e.y),
            (None, EventKind::Drag) => "dragged".into(),
        };
        pool.push(Selected {
            at_ms: e.at_ms,
            anchor_ms: e.at_ms,
            reason,
            action: Some(action),
            point: Some([e.x, e.y]),
            mark_ids: vec![],
            said: String::new(),
        });
    }
    for w in input.words {
        if !STRONG_CUES.contains(&normalize(&w.w).as_str()) {
            continue;
        }
        let lo = w.start_ms.saturating_sub(cfg.cue_before_ms);
        let hi = w.start_ms + cfg.cue_after_ms;
        let at_ms = transitions
            .iter()
            .find(|t| t.start_ms >= lo && t.start_ms <= hi)
            .map(|t| t.settled_ms)
            .unwrap_or(w.start_ms + cfg.cue_fallback_ms);
        pool.push(Selected {
            at_ms,
            anchor_ms: w.start_ms,
            reason: Reason::Cue,
            action: None,
            point: None,
            mark_ids: vec![],
            said: String::new(),
        });
    }
    for t in &transitions {
        pool.push(Selected {
            at_ms: t.settled_ms,
            anchor_ms: t.start_ms,
            reason: Reason::Scene,
            action: Some("scene changed".into()),
            point: None,
            mark_ids: vec![],
            said: String::new(),
        });
    }

    let mut leftovers = Vec::new();
    for s in pool {
        if !accept(&s, &chosen, input, cfg) {
            continue;
        }
        if chosen.len() >= budget {
            leftovers.push(s);
        } else {
            chosen.push(s);
        }
    }
    fill_coverage(&mut chosen, &mut leftovers, input, cfg, cap);

    if chosen.is_empty() {
        chosen.push(Selected {
            at_ms: input.duration_ms / 2,
            anchor_ms: 0,
            reason: Reason::Midpoint,
            action: None,
            point: None,
            mark_ids: vec![],
            said: String::new(),
        });
    }
    chosen.sort_by_key(|s| s.at_ms);
    assign_words(&mut chosen, input.words, cfg);
    chosen
}
```

- [x] Run `cargo test -p comeback-core keyframes`. Expect `10 passed`. If `coverage_fills_long_gaps_from_leftovers` fails, check that leftovers are only budget overflow: the `accept` check must run before the budget check, as written.
- [x] Commit: `git add -A && git commit -m "feat(core): select keyframes from marks, clicks, cues and scene changes"`

---

### Task 6: Text, badge and ripple drawing

**Files:**
- Create: `crates/comeback-core/src/render.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod render;`)

- [x] Create `render.rs` holding only the tests:

```rust
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
```

- [x] Run `cargo test -p comeback-core render`. Expect a compile FAIL: `cannot find function draw_badge`.
- [x] Add above the tests:

```rust
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
```

- [x] Run `cargo test -p comeback-core render`. Expect `3 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): draw keyframe badges and click ripples"`

---

### Task 7: Contact sheet

**Files:**
- Create: `crates/comeback-core/src/sheet.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod sheet;`)

Layout rule: rows are filled greedily at a 400 px target height while the row fits in 1600 px, and every row gets at least one frame. Each row except the last is then scaled to exactly 1600 px wide. The last row keeps the smaller of its fitted height and 400 px. The canvas is as wide as the widest row. The whole layout is then scaled down by `min(1, 1600 / height, sqrt(1_560_000 / (width * height)))`.

- [x] Create `sheet.rs` holding only the tests:

```rust
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
```

- [x] Run `cargo test -p comeback-core sheet`. Expect a compile FAIL: `cannot find function layout`.
- [x] Add above the tests:

```rust
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
```

- [x] Run `cargo test -p comeback-core sheet`. Expect `4 passed`. If a containment assertion fails by 1 px, the cause is floor rounding on `x + w`. Fix it by clamping each cell's `w` to `canvas_w - x` (and `h` likewise) after scaling. Leave the test as written.
- [x] Commit: `git add -A && git commit -m "feat(core): lay out and render the contact sheet"`

---

### Task 8: Paste text and summary.md

**Files:**
- Create: `crates/comeback-core/src/summary.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod summary;`)

- [x] Create `summary.rs` holding only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn manifest() -> Manifest {
        let kf = |n, at_ms, action: Option<&str>, said: &str| Keyframe {
            n,
            at_ms,
            reason: Reason::Click,
            file: format!("frames/frame-{n:02}.png"),
            said: said.into(),
            action: action.map(Into::into),
        };
        Manifest {
            schema: SCHEMA,
            id: "comeback-1".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            duration_ms: 33_000,
            backend: "x11".into(),
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 1920, 1080],
                scale: 1.0,
                app: Some("firefox".into()),
                title: Some("Settings".into()),
            },
            keyframes: vec![
                kf(1, 4_000, Some("clicked \"New tab\""), "Ctrl T makes a tab."),
                kf(2, 18_000, None, "And then it jumps."),
            ],
            marks: vec![],
            events: vec![],
            page: vec![],
            transcript: None,
            notes: vec![],
        }
    }

    #[test]
    fn paste_text_matches_the_spec_format() {
        let got = paste_text(&manifest(), Path::new("/tmp/b.comeback"));
        let want = "\
[comeback 0:33 · firefox \"Settings\" · /tmp/b.comeback]
- #1 (0:04) [clicked \"New tab\"] \"Ctrl T makes a tab.\"
- #2 (0:18) \"And then it jumps.\"
Contact sheet: /tmp/b.comeback/contact-sheet.png
";
        assert_eq!(got, want);
    }

    #[test]
    fn notes_come_after_the_header() {
        let mut m = manifest();
        m.notes.push("transcription failed (no model)".into());
        let got = paste_text(&m, Path::new("/b"));
        assert_eq!(got.lines().nth(1), Some("Note: transcription failed (no model)"));
    }

    #[test]
    fn summary_lists_frame_paths() {
        let got = summary_md(&manifest(), Path::new("/b"));
        assert!(got.ends_with("\nFrames:\n- #1 /b/frames/frame-01.png\n- #2 /b/frames/frame-02.png\n"));
    }
}
```

- [x] Run `cargo test -p comeback-core summary`. Expect a compile FAIL: `cannot find function paste_text`.
- [x] Add above the tests:

```rust
use std::fmt::Write;
use std::path::Path;

use crate::model::Manifest;
use crate::time::fmt_ms;

/// The text that goes on the clipboard and at the top of summary.md.
pub fn paste_text(m: &Manifest, dir: &Path) -> String {
    let target = match (&m.target.app, &m.target.title) {
        (Some(a), Some(t)) => format!(" · {a} \"{t}\""),
        (Some(a), None) => format!(" · {a}"),
        _ => String::new(),
    };
    let mut s = String::new();
    writeln!(s, "[comeback {}{} · {}]", fmt_ms(m.duration_ms), target, dir.display()).unwrap();
    for n in &m.notes {
        writeln!(s, "Note: {n}").unwrap();
    }
    for k in &m.keyframes {
        let mut line = format!("- #{} ({})", k.n, fmt_ms(k.at_ms));
        if let Some(a) = &k.action {
            write!(line, " [{a}]").unwrap();
        }
        if !k.said.is_empty() {
            write!(line, " \"{}\"", k.said).unwrap();
        }
        writeln!(s, "{line}").unwrap();
    }
    writeln!(s, "Contact sheet: {}", dir.join("contact-sheet.png").display()).unwrap();
    s
}

/// Paste text plus the absolute path of every keyframe.
pub fn summary_md(m: &Manifest, dir: &Path) -> String {
    let mut s = paste_text(m, dir);
    s.push_str("\nFrames:\n");
    for k in &m.keyframes {
        writeln!(s, "- #{} {}", k.n, dir.join(&k.file).display()).unwrap();
    }
    s
}
```

- [x] Run `cargo test -p comeback-core summary`. Expect `3 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): render paste text and summary.md"`

---

### Task 9: Compile step

**Files:**
- Create: `crates/comeback-core/src/compile.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod compile;`)

- [x] Create `compile.rs` holding only the test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventKind, TargetKind};
    use image::Rgb;

    #[test]
    fn writes_a_complete_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("comeback-1.comeback");
        let input = CompileInput {
            id: "comeback-1".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            backend: "test".into(),
            target: Target { kind: TargetKind::Screen, rect: [0, 0, 640, 360], scale: 1.0, app: None, title: None },
            duration_ms: 10_000,
            events: vec![Event {
                at_ms: 2_000,
                kind: EventKind::Click,
                x: 0.5,
                y: 0.5,
                to: None,
                button: None,
                label: None,
            }],
            marks: vec![Mark {
                id: 1,
                at_ms: 6_000,
                end_ms: 6_500,
                bbox: [0.1, 0.1, 0.2, 0.2],
                strokes: vec![],
                keyframe: None,
            }],
            words: vec![],
            transcript_model: None,
            candidates: vec![],
            page: vec![],
            notes: vec![],
        };
        let m = compile(input, &KeyframeConfig::default(), |_| Ok(RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]))), &out).unwrap();
        assert_eq!(m.keyframes.len(), 2);
        assert_eq!(m.marks[0].keyframe, Some(2));
        assert!(m.transcript.is_none());
        for f in ["comeback.json", "summary.md", "contact-sheet.png", "frames/frame-01.png", "frames/frame-02.png"] {
            assert!(out.join(f).is_file(), "missing {f}");
        }
        let back: Manifest = serde_json::from_slice(&std::fs::read(out.join("comeback.json")).unwrap()).unwrap();
        assert_eq!(back, m);
    }
}
```

- [x] Run `cargo test -p comeback-core compile`. Expect a compile FAIL: `cannot find struct CompileInput`.
- [x] Add above the test:

```rust
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use image::RgbImage;

use crate::keyframes::{select, Candidate, KeyframeConfig, SelectInput};
use crate::model::{Event, Keyframe, Manifest, Mark, PageEvent, Target, Transcript, Word, SCHEMA};
use crate::render::{draw_badge, draw_ripple};
use crate::sheet::render_sheet;
use crate::summary::summary_md;

pub struct CompileInput {
    pub id: String,
    pub started_at: String,
    pub backend: String,
    pub target: Target,
    pub duration_ms: u64,
    pub events: Vec<Event>,
    pub marks: Vec<Mark>,
    pub words: Vec<Word>,
    /// `None` means no transcript (no audio, or transcription failed).
    pub transcript_model: Option<String>,
    pub candidates: Vec<Candidate>,
    pub page: Vec<PageEvent>,
    pub notes: Vec<String>,
}

/// Selects keyframes, asks `frame_at` for each one at full size, and writes the bundle to `out`.
pub fn compile(
    input: CompileInput,
    cfg: &KeyframeConfig,
    mut frame_at: impl FnMut(u64) -> Result<RgbImage>,
    out: &Path,
) -> Result<Manifest> {
    let selected = select(
        &SelectInput {
            duration_ms: input.duration_ms,
            events: &input.events,
            marks: &input.marks,
            words: &input.words,
            candidates: &input.candidates,
        },
        cfg,
    );
    fs::create_dir_all(out.join("frames")).with_context(|| format!("creating {}", out.display()))?;

    let mut marks = input.marks;
    let mut frames = Vec::with_capacity(selected.len());
    let mut keyframes = Vec::with_capacity(selected.len());
    for (i, s) in selected.iter().enumerate() {
        let n = i as u32 + 1;
        let mut img = frame_at(s.at_ms).with_context(|| format!("frame at {} ms", s.at_ms))?;
        if let Some([x, y]) = s.point {
            draw_ripple(&mut img, x, y);
        }
        draw_badge(&mut img, n, s.at_ms);
        let file = format!("frames/frame-{n:02}.png");
        img.save(out.join(&file)).with_context(|| format!("writing {file}"))?;
        for m in marks.iter_mut().filter(|m| s.mark_ids.contains(&m.id)) {
            m.keyframe = Some(n);
        }
        keyframes.push(Keyframe { n, at_ms: s.at_ms, reason: s.reason, file, said: s.said.clone(), action: s.action.clone() });
        frames.push(img);
    }
    render_sheet(&frames).save(out.join("contact-sheet.png")).context("writing contact-sheet.png")?;

    let manifest = Manifest {
        schema: SCHEMA,
        id: input.id,
        started_at: input.started_at,
        duration_ms: input.duration_ms,
        backend: input.backend,
        target: input.target,
        keyframes,
        marks,
        events: input.events,
        page: input.page,
        transcript: input.transcript_model.map(|model| Transcript { model, words: input.words }),
        notes: input.notes,
    };
    fs::write(out.join("comeback.json"), serde_json::to_vec_pretty(&manifest)?).context("writing comeback.json")?;
    fs::write(out.join("summary.md"), summary_md(&manifest, out)).context("writing summary.md")?;
    Ok(manifest)
}
```

- [x] Run `cargo test -p comeback-core compile`. Expect `1 passed`.
- [x] Commit: `git add -A && git commit -m "feat(core): compile a timeline into a bundle"`

---

### Task 10: Bundle loading and `latest`

**Files:**
- Create: `crates/comeback-core/src/bundle.rs`
- Modify: `crates/comeback-core/src/lib.rs` (add `pub mod bundle;`)

- [x] Create `bundle.rs` holding only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_picks_the_newest_capture() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["comeback-100.comeback", "comeback-900.comeback", "comeback-50.comeback", "notes.txt"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        assert_eq!(latest(dir.path()).unwrap(), dir.path().join("comeback-900.comeback"));
        assert_eq!(resolve("latest", dir.path()).unwrap(), dir.path().join("comeback-900.comeback"));
        assert_eq!(resolve("/x/y.comeback", dir.path()).unwrap(), PathBuf::from("/x/y.comeback"));
    }

    #[test]
    fn latest_errors_on_an_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = latest(dir.path()).unwrap_err().to_string();
        assert!(err.starts_with("no captures in"), "{err}");
    }

    #[test]
    fn load_rejects_other_schemas() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("comeback.json"), r#"{"schema": 2}"#).unwrap();
        let err = load(dir.path()).unwrap_err().to_string();
        assert_eq!(err, "unsupported bundle schema 2 (this comeback reads schema 1)");
    }
}
```

- [x] Run `cargo test -p comeback-core bundle`. Expect a compile FAIL: `cannot find function latest`.
- [x] Add above the tests:

```rust
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::model::{Manifest, SCHEMA};

pub fn load(dir: &Path) -> Result<Manifest> {
    let path = dir.join("comeback.json");
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let raw: serde_json::Value = serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let schema = raw.get("schema").and_then(|v| v.as_u64()).unwrap_or(0);
    if schema != SCHEMA as u64 {
        bail!("unsupported bundle schema {schema} (this comeback reads schema {SCHEMA})");
    }
    serde_json::from_value(raw).with_context(|| format!("parsing {}", path.display()))
}

/// The capture directory with the largest `comeback-<ms>.comeback` timestamp.
pub fn latest(captures: &Path) -> Result<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(captures) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(ms) = name
                .strip_prefix("comeback-")
                .and_then(|r| r.strip_suffix(".comeback"))
                .and_then(|r| r.parse::<u64>().ok())
            else {
                continue;
            };
            if best.as_ref().is_none_or(|(b, _)| ms > *b) {
                best = Some((ms, e.path()));
            }
        }
    }
    match best {
        Some((_, p)) => Ok(p),
        None => bail!("no captures in {}", captures.display()),
    }
}

/// `latest` or a path.
pub fn resolve(arg: &str, captures: &Path) -> Result<PathBuf> {
    if arg == "latest" { latest(captures) } else { Ok(PathBuf::from(arg)) }
}
```

- [x] Run `cargo test -p comeback-core bundle`. Expect `3 passed`.
- [x] Run `cargo test -p comeback-core`. Expect every core test to pass (29 total).
- [x] Commit: `git add -A && git commit -m "feat(core): load bundles and resolve latest"`

---

### Task 11: CLI with `demo` and `inspect`

**Files:**
- Modify: `crates/comeback/src/main.rs`
- Create: `crates/comeback/src/demo.rs`, `crates/comeback/tests/cli.rs`

The captures directory is `$COMEBACK_CAPTURES_DIR` when set (tests use this), else `$XDG_DATA_HOME/comeback/captures`, else `$HOME/.local/share/comeback/captures`. `demo` without `--out` writes `comeback-<unix_ms>.comeback` there.

- [x] Create `crates/comeback/tests/cli.rs`:

```rust
use std::process::Command;

fn comeback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_comeback"))
}

#[test]
fn demo_then_inspect() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("comeback-123.comeback");

    let demo = comeback().args(["demo", "--out"]).arg(&out).output().unwrap();
    assert!(demo.status.success(), "{}", String::from_utf8_lossy(&demo.stderr));
    let text = String::from_utf8(demo.stdout).unwrap();
    assert!(text.starts_with("[comeback 0:20 · demo \"Settings\" · "), "{text}");
    assert!(text.contains("[clicked \"New tab\"]"), "{text}");
    assert!(text.contains("[marked]"), "{text}");
    for f in ["comeback.json", "summary.md", "contact-sheet.png", "frames/frame-01.png"] {
        assert!(out.join(f).is_file(), "missing {f}");
    }

    let json = comeback().args(["inspect", "--json"]).arg(&out).output().unwrap();
    assert!(json.status.success());
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["durationMs"], 20_000);

    let latest = comeback()
        .args(["inspect", "latest"])
        .env("COMEBACK_CAPTURES_DIR", dir.path())
        .output()
        .unwrap();
    assert!(latest.status.success());
    assert_eq!(String::from_utf8(latest.stdout).unwrap(), text);
}

#[test]
fn inspect_reports_missing_captures() {
    let dir = tempfile::tempdir().unwrap();
    let out = comeback().args(["inspect"]).env("COMEBACK_CAPTURES_DIR", dir.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no captures in"));
}
```

- [x] Run `cargo test -p comeback --test cli`. Expect FAIL: `demo_then_inspect` panics because the binary has no subcommands (the status is not success).
- [x] Create `crates/comeback/src/demo.rs`:

```rust
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
```

- [x] Replace `crates/comeback/src/main.rs`:

```rust
mod demo;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::{Parser, Subcommand};
use comeback_core::{bundle, summary};

#[derive(Parser)]
#[command(name = "comeback", version, about = "Show an agent the problem: screen, voice and ink in one bundle")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a synthetic bundle without capture hardware and print its paste text.
    Demo {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Print a bundle's paste text, or its manifest with --json.
    Inspect {
        #[arg(default_value = "latest")]
        bundle: String,
        #[arg(long)]
        json: bool,
    },
}

fn captures_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("COMEBACK_CAPTURES_DIR") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share"));
    data.join("comeback/captures")
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Demo { out } => {
            let id = format!("comeback-{}", now_ms());
            let out = out.unwrap_or_else(|| captures_dir().join(format!("{id}.comeback")));
            let m = demo::run(&out, id)?;
            print!("{}", summary::paste_text(&m, &out));
        }
        Cmd::Inspect { bundle: arg, json } => {
            let dir = bundle::resolve(&arg, &captures_dir())?;
            let m = bundle::load(&dir)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                print!("{}", summary::paste_text(&m, &dir));
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("comeback: {e:#}");
            ExitCode::from(1)
        }
    }
}
```

- [x] Run `cargo test -p comeback --test cli`. Expect `2 passed`.
- [x] Run `cargo run -q -p comeback -- demo --out /tmp/comeback-demo.comeback`. Expect exactly:

```text
[comeback 0:20 · demo "Settings" · /tmp/comeback-demo.comeback]
- #1 (0:03) [clicked "New tab"] "Clicking new tab does nothing."
- #2 (0:05) [scene changed] "Then this panel shows up."
- #3 (0:09) [marked] "This label is wrong."
- #4 (0:12) "And now everything goes white."
Contact sheet: /tmp/comeback-demo.comeback/contact-sheet.png
```

  Open `/tmp/comeback-demo.comeback/contact-sheet.png` and confirm that the frames are in time order and each one has a `#N M:SS` badge, the first shows a yellow ripple on the blue button, and one shows the red circle.
- [x] Commit: `git add -A && git commit -m "feat(cli): add demo and inspect commands"`

---

### Task 12: CI and lint gate

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] Create `.github/workflows/ci.yml`:

```yaml
name: ci
on:
  push:
    branches: [main]
  pull_request:
jobs:
  test:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
      - run: cargo run -q -p comeback -- demo --out "$RUNNER_TEMP/demo.comeback"
```

- [ ] Run `cargo fmt --all`, then `cargo clippy --workspace --all-targets -- -D warnings`. Expect no warnings. Fix any clippy finding in place without changing behavior, and rerun `cargo test --workspace` (expect all 31 tests to pass).
- [ ] Commit: `git add -A && git commit -m "ci: add fmt, clippy, test and demo checks"`

---

## Spec coverage

| Spec item | Task |
|---|---|
| Bundle format, `comeback.json` schema 1 | 2, 9 |
| Keyframe rules: events only, mark grouping, cue snapping and novelty, budget, coverage, clause ownership, midpoint | 5 |
| `KeyframeConfig` holds all constants | 5 |
| `#N M:SS` badge, click ripple | 6 |
| Contact sheet: time order, justified rows, 6 px gutters, max 1600 px, about 1.56 MP | 7 |
| Paste text format, `Note:` lines, `summary.md` | 8 |
| Transcription failure gives `transcript: null` plus a note | 9 (`transcript_model: None`, `notes`) |
| `inspect [--json]`, `latest` | 10, 11 |
| `demo`, CI runs it | 11, 12 |
| `comeback-core` builds with no system libraries | 1 (deps: serde, image, ab_glyph, anyhow only) |

## Follow-on plans (each gets its own `docs/plans/` file after this one lands)

2. **X11 capture and daemon:** GStreamer pipeline (`ximagesrc`, mic, tee to MP4 and the candidate sampler), XInput2 clicks, `XGrabKey` hotkey, ink overlay, whisper-rs, IPC (`toggle`, `cancel`), tray, clipboard, `record`, `rebuild`, `model fetch`, `zoom`, `frames`, the `Video:` line in `summary.md`, crash recovery, retention and `pin`. Needs the apt packages listed in the spec first.
3. **Wayland backend:** ScreenCast and GlobalShortcuts portals with a restore token, freeze-and-draw ink, the `wl-copy` fallback, `docs/testing-wayland.md`.
4. **Dev-page SDK, export and skill:** `comeback.js` and `POST /events` on 127.0.0.1:7317, `export [--crop]`, `skills/comeback/SKILL.md`, `comeback skill --install`.
