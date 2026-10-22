//! Turns raw pointer samples into the manifest's clicks, drags and ink marks.
//! Pure logic: the X11 reader feeds it, tests drive it directly.

use comeback_core::model::{Event, EventKind, Mark};

/// One raw pointer sample, in global screen pixels, stamped in capture milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Raw {
    Down {
        t: u64,
        button: u8,
        x: i32,
        y: i32,
        shift: bool,
    },
    Move {
        t: u64,
        x: i32,
        y: i32,
    },
    Up {
        t: u64,
        button: u8,
        x: i32,
        y: i32,
    },
}

/// What the ink overlay should do now. Coordinates are global screen pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ink {
    Begin([i32; 2]),
    Point([i32; 2]),
    End,
}

/// A press that moves less than this many pixels is a click, not a drag.
const DRAG_PX: f64 = 8.0;
/// Ink points closer than this to the previous point are dropped.
const INK_STEP_PX: f64 = 2.0;

struct Press {
    t: u64,
    button: u8,
    at: [i32; 2],
    ink: Option<Vec<[i32; 3]>>,
}

pub struct Gestures {
    target: [i32; 4],
    events: Vec<Event>,
    marks: Vec<Mark>,
    press: Option<Press>,
}

fn dist(a: [i32; 2], b: [i32; 2]) -> f64 {
    f64::from(a[0] - b[0]).hypot(f64::from(a[1] - b[1]))
}

impl Gestures {
    pub fn new(target: [i32; 4]) -> Self {
        Self {
            target,
            events: Vec::new(),
            marks: Vec::new(),
            press: None,
        }
    }

    fn inside(&self, x: i32, y: i32) -> bool {
        let [tx, ty, tw, th] = self.target;
        x >= tx && y >= ty && x < tx + tw && y < ty + th
    }

    /// Fractions of the target, clamped to its edges.
    fn frac(&self, x: i32, y: i32) -> [f64; 2] {
        let [tx, ty, tw, th] = self.target;
        [
            (f64::from(x - tx) / f64::from(tw)).clamp(0.0, 1.0),
            (f64::from(y - ty) / f64::from(th)).clamp(0.0, 1.0),
        ]
    }

    /// True while a Shift-left press inside the target is down: a stroke the application must
    /// not receive. Only such a press is kept from the application.
    pub fn has_ink_press(&self) -> bool {
        self.press.as_ref().is_some_and(|p| p.ink.is_some())
    }

    pub fn feed(&mut self, raw: Raw) -> Option<Ink> {
        match raw {
            Raw::Down {
                t,
                button,
                x,
                y,
                shift,
            } => {
                if !(1..=3).contains(&button) || self.press.is_some() || !self.inside(x, y) {
                    return None;
                }
                let ink = (shift && button == 1).then(|| vec![[x, y, t as i32]]);
                let drawing = ink.is_some();
                self.press = Some(Press {
                    t,
                    button,
                    at: [x, y],
                    ink,
                });
                drawing.then_some(Ink::Begin([x, y]))
            }
            Raw::Move { t, x, y } => {
                let press = self.press.as_mut()?;
                let points = press.ink.as_mut()?;
                let last = points.last().expect("a stroke starts with a point");
                if dist([last[0], last[1]], [x, y]) < INK_STEP_PX {
                    return None;
                }
                points.push([x, y, t as i32]);
                Some(Ink::Point([x, y]))
            }
            Raw::Up { t, button, x, y } => {
                if self.press.as_ref().is_none_or(|p| p.button != button) {
                    return None;
                }
                let mut press = self.press.take().expect("checked above");
                match press.ink.take() {
                    Some(mut points) => {
                        let last = points.last().expect("a stroke starts with a point");
                        if dist([last[0], last[1]], [x, y]) >= INK_STEP_PX {
                            points.push([x, y, t as i32]);
                        }
                        if points.len() >= 2 {
                            self.push_mark(&points, t);
                        } else {
                            self.push_click_or_drag(&press, [x, y]);
                        }
                        Some(Ink::End)
                    }
                    None => {
                        self.push_click_or_drag(&press, [x, y]);
                        None
                    }
                }
            }
        }
    }

    fn push_click_or_drag(&mut self, press: &Press, end: [i32; 2]) {
        let [x, y] = self.frac(press.at[0], press.at[1]);
        let drag = press.button == 1 && dist(press.at, end) >= DRAG_PX;
        let button = match press.button {
            1 => "left",
            2 => "middle",
            _ => "right",
        };
        self.events.push(Event {
            at_ms: press.t,
            kind: if drag {
                EventKind::Drag
            } else {
                EventKind::Click
            },
            x,
            y,
            to: drag.then(|| self.frac(end[0], end[1])),
            button: Some(button.into()),
            label: None,
        });
    }

    fn push_mark(&mut self, points: &[[i32; 3]], end_ms: u64) {
        let strokes: Vec<[f64; 3]> = points
            .iter()
            .map(|p| {
                let [fx, fy] = self.frac(p[0], p[1]);
                [fx, fy, f64::from(p[2])]
            })
            .collect();
        let bbox = strokes.iter().fold([1.0f64, 1.0, 0.0, 0.0], |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        });
        self.marks.push(Mark {
            id: self.marks.len() as u32 + 1,
            at_ms: points[0][2] as u64,
            end_ms,
            bbox,
            strokes: vec![strokes],
            keyframe: None,
        });
    }

    /// Closes any stroke still being drawn and returns what was recorded.
    pub fn finish(mut self) -> (Vec<Event>, Vec<Mark>) {
        if let Some(Press {
            ink: Some(points), ..
        }) = self.press.take()
            && points.len() >= 2
        {
            let end = points.last().expect("non-empty")[2] as u64;
            self.push_mark(&points, end);
        }
        (self.events, self.marks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: [i32; 4] = [100, 50, 800, 400];

    fn down(t: u64, x: i32, y: i32, shift: bool) -> Raw {
        Raw::Down {
            t,
            button: 1,
            x,
            y,
            shift,
        }
    }
    fn up(t: u64, x: i32, y: i32) -> Raw {
        Raw::Up { t, button: 1, x, y }
    }

    fn run(raws: &[Raw]) -> (Vec<Event>, Vec<Mark>) {
        let mut g = Gestures::new(T);
        for r in raws {
            g.feed(*r);
        }
        g.finish()
    }

    #[test]
    fn a_still_press_is_a_click_in_target_fractions() {
        let (events, marks) = run(&[down(1000, 500, 250, false), up(1060, 500, 251)]);
        assert!(marks.is_empty());
        assert_eq!(events.len(), 1);
        let e = &events[0];
        assert_eq!(e.kind, EventKind::Click);
        assert_eq!(e.at_ms, 1000);
        assert!(
            (e.x - 0.5).abs() < 1e-9 && (e.y - 0.5).abs() < 1e-9,
            "{e:?}"
        );
        assert_eq!(e.button.as_deref(), Some("left"));
        assert_eq!(e.to, None);
    }

    #[test]
    fn a_long_press_move_is_a_drag_with_an_end_point() {
        let (events, _) = run(&[
            down(1000, 200, 100, false),
            Raw::Move {
                t: 1100,
                x: 400,
                y: 100,
            },
            up(1200, 500, 100),
        ]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::Drag);
        assert_eq!(events[0].at_ms, 1000);
        let to = events[0].to.unwrap();
        assert!(
            (to[0] - 0.5).abs() < 1e-9 && (to[1] - 0.125).abs() < 1e-9,
            "{to:?}"
        );
    }

    #[test]
    fn right_and_middle_clicks_are_named() {
        let mut g = Gestures::new(T);
        g.feed(Raw::Down {
            t: 10,
            button: 3,
            x: 300,
            y: 100,
            shift: false,
        });
        g.feed(Raw::Up {
            t: 40,
            button: 3,
            x: 300,
            y: 100,
        });
        g.feed(Raw::Down {
            t: 50,
            button: 2,
            x: 300,
            y: 100,
            shift: false,
        });
        g.feed(Raw::Up {
            t: 80,
            button: 2,
            x: 300,
            y: 100,
        });
        let (events, _) = g.finish();
        let names: Vec<_> = events.iter().map(|e| e.button.clone().unwrap()).collect();
        assert_eq!(names, ["right", "middle"]);
    }

    #[test]
    fn scroll_wheel_buttons_are_ignored() {
        let mut g = Gestures::new(T);
        for b in [4u8, 5, 6, 7] {
            g.feed(Raw::Down {
                t: 10,
                button: b,
                x: 300,
                y: 100,
                shift: false,
            });
            g.feed(Raw::Up {
                t: 11,
                button: b,
                x: 300,
                y: 100,
            });
        }
        assert!(g.finish().0.is_empty());
    }

    #[test]
    fn presses_outside_the_target_are_dropped() {
        let (events, marks) = run(&[
            down(10, 50, 100, false),
            up(40, 50, 100),
            down(50, 500, 900, true),
            Raw::Move {
                t: 60,
                x: 520,
                y: 920,
            },
            up(70, 520, 920),
        ]);
        assert!(events.is_empty());
        assert!(marks.is_empty());
    }

    #[test]
    fn a_drag_ending_outside_is_clamped() {
        let (events, _) = run(&[down(10, 800, 250, false), up(400, 2000, 250)]);
        assert_eq!(events[0].kind, EventKind::Drag);
        assert_eq!(events[0].to.unwrap()[0], 1.0);
    }

    #[test]
    fn shift_drag_draws_ink_not_a_drag() {
        let mut g = Gestures::new(T);
        assert_eq!(
            g.feed(down(2000, 200, 100, true)),
            Some(Ink::Begin([200, 100]))
        );
        assert_eq!(
            g.feed(Raw::Move {
                t: 2050,
                x: 260,
                y: 140
            }),
            Some(Ink::Point([260, 140]))
        );
        assert_eq!(
            g.feed(Raw::Move {
                t: 2100,
                x: 330,
                y: 190
            }),
            Some(Ink::Point([330, 190]))
        );
        assert_eq!(g.feed(up(2150, 330, 190)), Some(Ink::End));
        let (events, marks) = g.finish();
        assert!(events.is_empty(), "ink is not a click or drag: {events:?}");
        assert_eq!(marks.len(), 1);
        let m = &marks[0];
        assert_eq!((m.id, m.at_ms, m.end_ms, m.keyframe), (1, 2000, 2150, None));
        assert_eq!(m.strokes.len(), 1);
        assert_eq!(m.strokes[0].len(), 3, "the release repeats the last point");
        assert_eq!(m.strokes[0][0], [0.125, 0.125, 2000.0]);
        assert_eq!(m.bbox, [0.125, 0.125, 0.2875, 0.35]);
    }

    #[test]
    fn jitter_under_two_pixels_adds_no_points() {
        let mut g = Gestures::new(T);
        g.feed(down(0, 300, 200, true));
        assert_eq!(
            g.feed(Raw::Move {
                t: 5,
                x: 301,
                y: 200
            }),
            None
        );
        g.feed(Raw::Move {
            t: 10,
            x: 340,
            y: 200,
        });
        g.feed(up(20, 340, 200));
        let (_, marks) = g.finish();
        assert_eq!(marks[0].strokes[0].len(), 2);
    }

    #[test]
    fn a_shift_click_without_movement_stays_a_click() {
        let mut g = Gestures::new(T);
        g.feed(down(500, 300, 200, true));
        g.feed(up(540, 300, 200));
        let (events, marks) = g.finish();
        assert!(marks.is_empty());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::Click);
    }

    #[test]
    fn only_a_shift_left_press_inside_the_target_is_an_ink_press() {
        let mut g = Gestures::new(T);
        g.feed(down(10, 300, 200, false));
        assert!(!g.has_ink_press(), "a plain press is the application's");
        g.feed(up(20, 300, 200));
        g.feed(down(30, 300, 200, true));
        assert!(g.has_ink_press());
        g.feed(up(40, 300, 200));
        assert!(!g.has_ink_press(), "released");
        g.feed(Raw::Down {
            t: 50,
            button: 3,
            x: 300,
            y: 200,
            shift: true,
        });
        assert!(!g.has_ink_press(), "Shift-right is not ink");
        g.feed(Raw::Up {
            t: 60,
            button: 3,
            x: 300,
            y: 200,
        });
        g.feed(down(70, 20, 20, true));
        assert!(
            !g.has_ink_press(),
            "outside the target is the application's"
        );
    }

    #[test]
    fn mark_ids_count_up_per_stroke() {
        let mut g = Gestures::new(T);
        for (i, t0) in [1000u64, 3000].into_iter().enumerate() {
            let y = 100 + 40 * i as i32;
            g.feed(down(t0, 200, y, true));
            g.feed(Raw::Move {
                t: t0 + 20,
                x: 260,
                y,
            });
            g.feed(up(t0 + 40, 300, y));
        }
        let (_, marks) = g.finish();
        assert_eq!(marks.iter().map(|m| m.id).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn an_unfinished_stroke_is_closed_by_finish() {
        let mut g = Gestures::new(T);
        g.feed(down(100, 200, 100, true));
        g.feed(Raw::Move {
            t: 150,
            x: 300,
            y: 160,
        });
        let (_, marks) = g.finish();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].end_ms, 150);
    }
}
