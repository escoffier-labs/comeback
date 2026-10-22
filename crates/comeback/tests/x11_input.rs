//! Pointer input against real client windows. Every test maps application windows that select
//! button events, so presses start the server's implicit grab, and injects input through XTest.
//! A test that only clicks the bare root window cannot see these bugs.

mod common;

use std::thread;
use std::time::Duration;

use common::*;

const TARGET: &str = "0,0,640,360";

struct Run {
    rec: Recorder,
    bundle: std::path::PathBuf,
    _tmp: tempfile::TempDir,
}

fn record(xvfb: &Xvfb) -> Run {
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("input.comeback");
    let mut cmd = comeback(Some(xvfb), &tmp.path().join("captures"));
    cmd.env("COMEBACK_AUDIO_SOURCE", "none")
        .args(["record", "--area", TARGET, "--no-clipboard", "--out"])
        .arg(&bundle);
    let mut rec = Recorder::spawn(cmd);
    rec.wait_for("comeback: recording 640x360");
    thread::sleep(Duration::from_millis(1000));
    Run {
        rec,
        bundle,
        _tmp: tmp,
    }
}

impl Run {
    fn finish(self) -> serde_json::Value {
        thread::sleep(Duration::from_millis(600));
        self.rec.sigint();
        let (code, _, stderr) = self.rec.finish();
        assert_eq!(code, Some(0), "stderr: {stderr}");
        manifest(&self.bundle)
    }
}

fn events(m: &serde_json::Value) -> Vec<(String, f64, f64)> {
    m["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_string(),
                e["x"].as_f64().unwrap(),
                e["y"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn frac(x: i16, y: i16) -> (f64, f64) {
    (f64::from(x) / 640.0, f64::from(y) / 360.0)
}

fn assert_click_at(got: &(String, f64, f64), x: i16, y: i16) {
    let (fx, fy) = frac(x, y);
    assert_eq!(got.0, "click", "{got:?}");
    assert!(
        (got.1 - fx).abs() < 0.01 && (got.2 - fy).abs() < 0.01,
        "{got:?} is not a click at {x},{y}"
    );
}

/// C1: a press on a normal application window starts an implicit pointer grab. Raw events must
/// still reach the recorder, so clicks on windows are recorded and later presses are not lost.
#[test]
fn clicks_on_an_application_window_are_recorded_and_still_reach_it() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let app = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let run = record(&xvfb);

    user.click(560, 330); // bare desktop
    thread::sleep(Duration::from_millis(200));
    let (ax, ay) = app.at(100, 60);
    user.click(ax, ay); // on the window: implicit grab
    thread::sleep(Duration::from_millis(200));
    let (bx, by) = app.at(250, 120);
    user.click(bx, by); // on the window again
    thread::sleep(Duration::from_millis(200));
    user.click(40, 40); // bare desktop again
    let m = run.finish();

    let got = events(&m);
    assert_eq!(got.len(), 4, "four clicks, no phantom drags: {got:?}");
    assert_click_at(&got[0], 560, 330);
    assert_click_at(&got[1], ax, ay);
    assert_click_at(&got[2], bx, by);
    assert_click_at(&got[3], 40, 40);
    assert_eq!(app.presses(), 2, "the application got its own clicks");
}

/// C1 for ink: a stroke over an application window ends at the release, not at some later click.
#[test]
fn an_ink_stroke_over_an_application_window_ends_where_the_button_is_released() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let app = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let run = record(&xvfb);

    let (x0, y0) = app.at(40, 40);
    let (x1, y1) = app.at(240, 140);
    user.ink((x0, y0), (x1, y1));
    // The button is up. Moving around and clicking elsewhere must not extend the stroke.
    user.move_to(600, 330);
    thread::sleep(Duration::from_millis(200));
    user.move_to(580, 20);
    thread::sleep(Duration::from_millis(200));
    user.click(30, 340);
    let m = run.finish();

    let marks = m["marks"].as_array().unwrap();
    assert_eq!(marks.len(), 1, "one stroke: {marks:?}");
    let stroke = marks[0]["strokes"][0].as_array().unwrap();
    let last = stroke.last().unwrap();
    let (fx, fy) = frac(x1, y1);
    assert!(
        (last[0].as_f64().unwrap() - fx).abs() < 0.02
            && (last[1].as_f64().unwrap() - fy).abs() < 0.02,
        "the stroke ends at the release point {fx:.3},{fy:.3}, got {last}"
    );
    let got = events(&m);
    assert_eq!(got.len(), 1, "the later click is recorded: {got:?}");
    assert_click_at(&got[0], 30, 340);
}

/// M5: an X error in the input thread must not throw away the gestures captured before it.
/// Losing the X server mid-recording is the realistic way to get one.
#[test]
fn losing_the_x_server_keeps_the_gestures_captured_so_far() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let mut xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let run = record(&xvfb);
    thread::sleep(Duration::from_millis(1200));
    user.click(200, 100);
    thread::sleep(Duration::from_millis(600));
    user.click(300, 200);
    thread::sleep(Duration::from_millis(1800));
    drop(user);
    xvfb.kill();
    // The encoder dies with the display; the recorder notices and wraps up what it has.
    let (code, _, stderr) = run.rec.finish();
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let m = manifest(&run.bundle);
    let got = events(&m);
    assert_eq!(got.len(), 2, "both clicks survive the X error: {got:?}");
    assert_click_at(&got[0], 200, 100);
    assert_click_at(&got[1], 300, 200);
    let notes = m["notes"].to_string();
    assert!(
        notes.contains("stopped early"),
        "the early stop is explained: {notes}"
    );
}

fn marks(m: &serde_json::Value) -> usize {
    m["marks"].as_array().unwrap().len()
}

/// Shift-left-drag inside the target is the user drawing, not an action on the application
/// underneath: the application must see no press, motion or release for it.
#[test]
fn shift_drag_inside_the_target_draws_ink_and_never_reaches_the_application() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let app = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let run = record(&xvfb);

    let from = app.at(40, 40);
    let to = app.at(240, 140);
    user.move_to(from.0, from.1);
    thread::sleep(Duration::from_millis(300));
    app.clear();
    user.ink(from, to);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        app.seen(),
        Vec::<Seen>::new(),
        "the application saw the ink stroke"
    );
    // The button is up again and the pointer is not stuck in a grab: a plain click works.
    let (cx, cy) = app.at(300, 150);
    user.click(cx, cy);
    assert_eq!(app.presses(), 1, "ordinary clicks work after the stroke");
    let m = run.finish();

    assert_eq!(marks(&m), 1, "{m}");
    let got = events(&m);
    assert_eq!(got.len(), 1, "only the later plain click: {got:?}");
    assert_click_at(&got[0], cx, cy);
}

/// Everything that is not "Shift-left press inside the target" is the application's, and is
/// recorded as the clicks and drags it always was.
#[test]
fn ordinary_clicks_and_drags_and_shift_input_outside_the_target_still_reach_the_application() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let inside = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let outside = AppWindow::open(&xvfb, (700, 100, 400, 300));
    let run = record(&xvfb);

    // a plain click and a plain drag inside the target
    let (px, py) = inside.at(50, 50);
    user.click(px, py);
    thread::sleep(Duration::from_millis(150));
    let (dx0, dy0) = inside.at(100, 100);
    let (dx1, dy1) = inside.at(300, 160);
    user.drag((dx0, dy0), (dx1, dy1));
    let seen = inside.seen();
    assert_eq!(
        seen.iter().filter(|s| matches!(s, Seen::Press(1))).count(),
        2,
        "{seen:?}"
    );
    assert_eq!(
        seen.iter()
            .filter(|s| matches!(s, Seen::Release(1)))
            .count(),
        2,
        "{seen:?}"
    );
    assert!(seen.contains(&Seen::Motion), "the drag's motion arrived");

    // Inside the target any Shift-left press is the draw gesture, even one that never moves:
    // it is recorded as a click and the application does not receive it.
    let (sx, sy) = inside.at(200, 30);
    user.move_to(sx, sy);
    thread::sleep(Duration::from_millis(300));
    inside.clear();
    user.shift_click(sx, sy);
    assert_eq!(
        inside.seen(),
        Vec::<Seen>::new(),
        "a Shift press inside is ink's"
    );

    // Shift-click and Shift-drag outside the target belong to the other application
    let from = outside.at(50, 50);
    let to = outside.at(250, 200);
    user.move_to(from.0, from.1);
    thread::sleep(Duration::from_millis(200));
    outside.clear();
    user.ink(from, to);
    let seen = outside.seen();
    assert_eq!(seen.first(), Some(&Seen::Press(1)), "{seen:?}");
    assert_eq!(seen.last(), Some(&Seen::Release(1)), "{seen:?}");
    assert!(
        seen.iter().filter(|s| **s == Seen::Motion).count() >= 5,
        "the drag's motion arrived: {seen:?}"
    );
    let (ox, oy) = outside.at(100, 250);
    user.move_to(ox, oy);
    thread::sleep(Duration::from_millis(300));
    outside.clear();
    user.shift_click(ox, oy);
    assert_eq!(outside.seen(), [Seen::Press(1), Seen::Release(1)]);
    let m = run.finish();

    assert_eq!(marks(&m), 0, "no ink outside the target: {m}");
    let got = events(&m);
    let kinds: Vec<_> = got.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(kinds, ["click", "drag", "click"], "{got:?}");
    assert_click_at(&got[0], px, py);
    assert_click_at(&got[2], sx, sy);
}

/// Ink lingers for seconds. It must not catch clicks meant for the window underneath.
#[test]
fn clicks_on_lingering_ink_reach_the_window_underneath() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let app = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let run = record(&xvfb);

    let from = app.at(40, 40);
    let to = app.at(240, 140);
    user.ink(from, to);
    let mid = app.at(140, 90);
    let (r, g, b) = user.pixel(mid.0, mid.1);
    assert!(
        r > 180 && g < 90 && b < 90,
        "the point we click is on red ink, got {r},{g},{b}"
    );
    user.move_to(mid.0, mid.1);
    thread::sleep(Duration::from_millis(300));
    app.clear();
    user.click(mid.0, mid.1);
    assert_eq!(
        app.seen(),
        [Seen::Press(1), Seen::Release(1)],
        "the click went through the ink"
    );
    let m = run.finish();
    assert_eq!(marks(&m), 1, "{m}");
    let got = events(&m);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_click_at(&got[0], mid.0, mid.1);
}

/// The pointer is only ever borrowed. After a normal stop, and even after the recorder is killed
/// in the middle of a stroke, the application gets its Shift-clicks again.
#[test]
fn the_pointer_is_given_back_after_stop_and_after_a_kill() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let app = AppWindow::open(&xvfb, (100, 80, 400, 200));
    let (x, y) = app.at(60, 60);
    let shift_click_arrives = |what: &str| {
        user.move_to(x, y);
        thread::sleep(Duration::from_millis(300));
        app.clear();
        user.shift_click(x, y);
        assert_eq!(
            app.seen(),
            [Seen::Press(1), Seen::Release(1)],
            "{what}: the application receives Shift-click again"
        );
    };

    // while recording, a Shift press inside the target is ink's
    let run = record(&xvfb);
    user.move_to(x, y);
    thread::sleep(Duration::from_millis(300));
    app.clear();
    user.shift_click(x, y);
    assert_eq!(app.seen(), Vec::<Seen>::new());
    run.finish();
    shift_click_arrives("after a normal stop");

    // killed with the stroke still being drawn: the button is still down when the grab holder dies
    let mut run = record(&xvfb);
    user.move_to(x, y);
    thread::sleep(Duration::from_millis(300));
    user.shift(true);
    thread::sleep(Duration::from_millis(100));
    user.button(true, 1);
    thread::sleep(Duration::from_millis(300));
    run.rec.child.kill().unwrap();
    run.rec.child.wait().unwrap();
    user.button(false, 1);
    user.shift(false);
    thread::sleep(Duration::from_millis(300));
    app.clear();
    shift_click_arrives("after the recorder was killed mid-stroke");
}

/// If another program already owns Shift+left-drag the recorder cannot draw. It must say so
/// where the user will see it, and still record video and voice.
#[test]
fn a_taken_shift_drag_grab_is_reported_not_silently_ignored() {
    use x11rb::protocol::xproto::{ButtonIndex, ConnectionExt as _, EventMask, GrabMode, ModMask};
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let rival = xvfb.client();
    rival.paint(BLUE);
    rival
        .conn
        .grab_button(
            false,
            rival.root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            ButtonIndex::M1,
            ModMask::SHIFT,
        )
        .unwrap()
        .check()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let bundle = tmp.path().join("noink.comeback");
    let out = comeback(Some(&xvfb), &tmp.path().join("captures"))
        .env("COMEBACK_AUDIO_SOURCE", "none")
        .args([
            "record",
            "--area",
            TARGET,
            "--duration",
            "2",
            "--no-clipboard",
            "--out",
        ])
        .arg(&bundle)
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{err}");
    assert!(
        err.contains("Shift+left-drag is already grabbed by another program"),
        "the user is told while recording starts: {err}"
    );
    let m = manifest(&bundle);
    assert!(
        m["notes"].to_string().contains("already grabbed"),
        "and the bundle says so: {}",
        m["notes"]
    );
    assert!(bundle.join("recording.mp4").is_file());
}
