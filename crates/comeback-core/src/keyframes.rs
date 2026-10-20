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
