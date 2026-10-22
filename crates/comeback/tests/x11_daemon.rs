//! Daemon, `toggle`, `cancel` and the Alt+Shift+S hotkey on a throwaway Xvfb display.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use common::*;

struct Rig {
    xvfb: Xvfb,
    captures: PathBuf,
    socket: PathBuf,
    dir: tempfile::TempDir,
}

fn rig() -> Rig {
    let xvfb = Xvfb::start();
    let dir = tempfile::tempdir().unwrap();
    // The daemon refuses a socket directory other users can enter.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let captures = dir.path().join("captures");
    let socket = dir.path().join("comeback.sock");
    Rig {
        xvfb,
        captures,
        socket,
        dir,
    }
}

impl Rig {
    fn command(&self) -> Command {
        let mut c = comeback(Some(&self.xvfb), &self.captures);
        c.env("COMEBACK_SOCKET", &self.socket)
            .env("COMEBACK_NOTIFY", "off")
            .env("COMEBACK_WHISPER_CLI", fake_whisper(self.dir.path()))
            .env("COMEBACK_WHISPER_MODEL", self.dir.path().join("model.bin"));
        std::fs::write(self.dir.path().join("model.bin"), b"x").unwrap();
        c
    }

    fn daemon(&self) -> Recorder {
        let mut cmd = self.command();
        cmd.arg("daemon");
        let mut d = Recorder::spawn(cmd);
        d.wait_for("comeback: daemon ready");
        d
    }

    /// `comeback <verb>` against the running daemon: (exit code, stdout, stderr).
    fn ctl(&self, verb: &str) -> (Option<i32>, String, String) {
        let out = self.command().arg(verb).output().unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        )
    }

    fn bundles(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(&self.captures)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.retain(|p| p.extension().is_some_and(|e| e == "comeback"));
        v.sort();
        v
    }
}

fn clipboard_text(xvfb: &Xvfb) -> Option<String> {
    let out = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .env("DISPLAY", &xvfb.display)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Presses Alt+Shift+S (keycodes of a default X keymap: Alt_L 64, Shift_L 50, s 39).
fn hotkey(user: &Client) {
    for code in [64u8, 50, 39] {
        user.fake(2, code, 0, 0);
    }
    thread::sleep(Duration::from_millis(60));
    for code in [39u8, 50, 64] {
        user.fake(3, code, 0, 0);
    }
}

/// Holds Alt+Shift+S down for `ms` like a finger does: the server autorepeats a held key.
fn hold_hotkey(user: &Client, ms: u64) {
    for code in [64u8, 50, 39] {
        user.fake(2, code, 0, 0);
    }
    thread::sleep(Duration::from_millis(ms));
    for code in [39u8, 50, 64] {
        user.fake(3, code, 0, 0);
    }
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if f() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn toggle_hotkey_and_cancel_drive_the_same_recorder() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let rig = rig();
    let user = rig.xvfb.client();
    user.paint(BLUE);
    let daemon = rig.daemon();

    assert_eq!(rig.ctl("status").1, "ok idle");
    let sock_mode = std::fs::metadata(&rig.socket).unwrap().permissions().mode() & 0o777;
    assert_eq!(sock_mode, 0o600, "the control socket is private");

    // toggle: start, click, stop
    let (code, out, err) = rig.ctl("toggle");
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with("ok recording comeback-"), "{out}");
    assert!(rig.ctl("status").1.starts_with("ok recording comeback-"));
    thread::sleep(Duration::from_millis(1500));
    user.click(320, 200);
    thread::sleep(Duration::from_millis(800));
    let (code, out, err) = rig.ctl("toggle");
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with("ok copied "), "{out}");
    let bundles = rig.bundles();
    assert_eq!(bundles.len(), 1, "{bundles:?}");
    let first = &bundles[0];
    for f in [
        "comeback.json",
        "recording.mp4",
        "audio.wav",
        "contact-sheet.png",
        "summary.md",
    ] {
        assert!(first.join(f).is_file(), "missing {f}");
    }
    let m = manifest(first);
    assert!(
        m["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "click"),
        "{m}"
    );
    assert_eq!(m["transcript"]["model"], "small.en");
    if on_path("xclip") {
        let text = std::fs::read_to_string(first.join("summary.md")).unwrap();
        let clip = clipboard_text(&rig.xvfb).expect("the clipboard has text");
        assert!(
            text.starts_with(clip.trim_end()),
            "clipboard {clip:?} vs summary {text:?}"
        );
        assert!(clip.starts_with("[comeback 0:0"), "{clip}");
    }

    // Alt+Shift+S starts, and stops once the 400 ms debounce has passed
    hotkey(&user);
    wait_until("the hotkey to start a recording", || {
        rig.ctl("status").1.starts_with("ok recording")
    });
    thread::sleep(Duration::from_millis(1500));
    hotkey(&user);
    wait_until("the hotkey to finish a bundle", || rig.bundles().len() == 2);
    // The bundle is written on a worker thread; the daemon reports `compiling` until it is done.
    wait_until("the daemon to go idle", || rig.ctl("status").1 == "ok idle");

    // a second toggle inside the debounce window does not stop a fresh recording
    let (_, out, _) = rig.ctl("toggle");
    assert!(out.starts_with("ok recording"), "{out}");
    let (_, again, _) = rig.ctl("toggle");
    assert!(again.starts_with("ok ignored"), "{again}");
    assert!(rig.ctl("status").1.starts_with("ok recording"));

    // cancel throws away only the running capture
    let before: Vec<_> = rig.bundles();
    let (code, out, _) = rig.ctl("cancel");
    assert_eq!((code, out.as_str()), (Some(0), "ok cancelled"));
    assert_eq!(rig.bundles(), before, "earlier bundles are untouched");
    assert!(
        leftovers(&rig.captures).is_empty(),
        "{:?}",
        leftovers(&rig.captures)
    );
    assert_eq!(rig.ctl("cancel").1, "ok nothing to cancel");
    assert!(before.iter().all(|b| b.join("recording.mp4").is_file()));

    // a second daemon on the same socket refuses to start
    let second = rig.command().arg("daemon").output().unwrap();
    assert_eq!(second.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&second.stderr).contains("already running"));
    assert_eq!(
        rig.ctl("status").1,
        "ok idle",
        "the first daemon is unharmed"
    );

    // SIGINT shuts it down and removes the socket
    daemon.sigint();
    let (code, _, stderr) = daemon.finish();
    assert_eq!(code, Some(0), "{stderr}");
    assert!(!rig.socket.exists());
}

#[test]
fn stopping_the_daemon_mid_recording_keeps_the_capture() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let rig = rig();
    rig.xvfb.client().paint(BLUE);
    let daemon = rig.daemon();
    assert!(rig.ctl("toggle").1.starts_with("ok recording"));
    thread::sleep(Duration::from_millis(1200));
    daemon.sigint();
    let (code, _, stderr) = daemon.finish();
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        rig.bundles().len(),
        1,
        "the recording was finished, not lost: {stderr}"
    );
    assert!(leftovers(&rig.captures).is_empty());
}

#[test]
fn clients_report_a_missing_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_comeback"))
        .arg("toggle")
        .env("COMEBACK_SOCKET", dir.path().join("none.sock"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("daemon is not running"));
}

#[test]
fn a_stale_socket_is_replaced_but_other_files_and_open_directories_are_refused() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let rig = rig();
    // stale: a socket file nobody listens on
    drop(std::os::unix::net::UnixListener::bind(&rig.socket).unwrap());
    assert!(rig.socket.exists());
    let daemon = rig.daemon();
    assert_eq!(rig.ctl("status").1, "ok idle");
    daemon.sigint();
    let _ = daemon.finish();

    // a regular file in the way is never deleted
    let file = rig.dir.path().join("not-a-socket");
    std::fs::write(&file, "mine").unwrap();
    let out = rig
        .command()
        .env("COMEBACK_SOCKET", &file)
        .arg("daemon")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a socket"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine");

    // a directory other users can enter is refused
    let open = rig.dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).unwrap();
    let out = rig
        .command()
        .env("COMEBACK_SOCKET", open.join("c.sock"))
        .arg("daemon")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("accessible to other users"));
    let _: &Path = &open;
}

/// `ok recording comeback-123 (456 ms)` without the changing part.
fn who(status: &str) -> String {
    status.split(" (").next().unwrap().to_string()
}

/// I2: a held hotkey autorepeats. One physical press is one toggle: it must not stop the
/// recording it just started, and presses queued behind a compile must not start another.
#[test]
fn holding_the_hotkey_toggles_exactly_once() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let rig = rig();
    let user = rig.xvfb.client();
    user.paint(BLUE);
    let daemon = rig.daemon();

    // A long hold starts one recording, and the autorepeat that follows leaves it running.
    hold_hotkey(&user, 1600);
    wait_until("the held hotkey to start a recording", || {
        rig.ctl("status").1.starts_with("ok recording")
    });
    let first = who(&rig.ctl("status").1);
    thread::sleep(Duration::from_millis(2000));
    assert_eq!(who(&rig.ctl("status").1), first, "still the same recording");
    assert!(
        rig.bundles().is_empty(),
        "nothing was stopped by the repeats"
    );

    // A long hold stops it once, and the repeats do not start another while it compiles.
    hold_hotkey(&user, 1600);
    wait_until("the held hotkey to finish the bundle", || {
        rig.bundles().len() == 1
    });
    wait_until("the daemon to go idle", || rig.ctl("status").1 == "ok idle");
    thread::sleep(Duration::from_millis(2000));
    assert_eq!(
        rig.ctl("status").1,
        "ok idle",
        "no surprise second recording"
    );
    assert_eq!(rig.bundles().len(), 1);
    assert!(
        leftovers(&rig.captures).is_empty(),
        "{:?}",
        leftovers(&rig.captures)
    );
    daemon.sigint();
    let (code, _, stderr) = daemon.finish();
    assert_eq!(code, Some(0), "{stderr}");
}

/// M4: a long transcription must not freeze the daemon. Status says what it is doing,
/// cancel says honestly that a finished capture cannot be cancelled, and presses meant for a
/// busy daemon are dropped instead of queued.
#[test]
fn status_cancel_and_toggle_answer_while_a_bundle_compiles() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let rig = rig();
    let user = rig.xvfb.client();
    user.paint(BLUE);
    let mut cmd = rig.command();
    cmd.arg("daemon")
        .env("COMEBACK_WHISPER_CLI", slow_whisper(rig.dir.path(), 6));
    let mut daemon = Recorder::spawn(cmd);
    daemon.wait_for("comeback: daemon ready");

    assert!(rig.ctl("toggle").1.starts_with("ok recording"));
    thread::sleep(Duration::from_millis(1500));
    let stopper = {
        let mut c = rig.command();
        c.arg("toggle");
        thread::spawn(move || c.output().unwrap())
    };
    let quick = |verb: &str| {
        let t = Instant::now();
        let r = rig.ctl(verb);
        assert!(
            t.elapsed() < Duration::from_millis(1500),
            "`{verb}` took {:?} while compiling",
            t.elapsed()
        );
        r
    };
    wait_until("status to report compiling", || {
        quick("status").1.starts_with("ok compiling comeback-")
    });

    let (code, out, err) = quick("cancel");
    assert_eq!(code, Some(1), "{out} {err}");
    assert!(
        err.contains("already recorded") && err.contains("kept"),
        "cancel says what it did not do: {err}"
    );
    let (code, out, err) = quick("toggle");
    assert_eq!(code, Some(1), "{out} {err}");
    assert!(err.contains("busy"), "{err}");
    hotkey(&user);
    thread::sleep(Duration::from_millis(300));
    assert!(quick("status").1.starts_with("ok compiling"));

    let done = stopper.join().unwrap();
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(text.trim().starts_with("ok copied "), "{text}");
    assert_eq!(rig.bundles().len(), 1, "the capture was not discarded");
    assert_eq!(rig.ctl("status").1, "ok idle");
    thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        rig.ctl("status").1,
        "ok idle",
        "the presses sent while busy did not start a recording later"
    );
    assert!(leftovers(&rig.captures).is_empty());
    daemon.sigint();
    let (code, _, stderr) = daemon.finish();
    assert_eq!(code, Some(0), "{stderr}");
}
