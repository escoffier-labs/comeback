//! What a failed or finished capture leaves on disk: raw media kept when a step fails, and
//! files only the owner can open, whatever the user's umask. Runs on a throwaway Xvfb display.

mod common;

use std::thread;
use std::time::Duration;

use common::*;

#[test]
fn an_unreadable_recording_is_kept_and_its_path_is_printed() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("never.comeback");
    let broken = tmp.path().join("broken-ffprobe");
    std::fs::write(&broken, "#!/bin/sh\necho 'cannot parse' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = comeback(Some(&xvfb), &captures)
        .env("COMEBACK_FFPROBE", &broken)
        .args([
            "record",
            "--area",
            "0,0,320,240",
            "--duration",
            "2",
            "--no-clipboard",
            "--out",
        ])
        .arg(&bundle)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    let kept: Vec<_> = std::fs::read_dir(&captures)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".recording-"))
        .collect();
    assert_eq!(kept.len(), 1, "the raw recording is not deleted: {err}");
    let dir = kept[0].path();
    assert!(
        err.contains(&dir.display().to_string()),
        "the error names {}: {err}",
        dir.display()
    );
    assert!(dir.join("recording.mp4").metadata().unwrap().len() > 1000);
    assert!(dir.join("audio.wav").metadata().unwrap().len() > 1000);
    assert!(!bundle.exists());
}

#[test]
fn captures_are_private_even_under_a_permissive_umask() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();

    // A captures directory comeback creates is its own to lock down.
    let fresh = tmp.path().join("fresh/captures");
    let mut cmd = comeback(Some(&xvfb), &fresh);
    cmd.args(["record", "--area", "0,0,320,240", "--no-clipboard"]);
    let mut rec = Recorder::spawn(under_open_umask(&cmd));
    rec.wait_for("comeback: recording");
    thread::sleep(Duration::from_millis(1000));
    let work = std::fs::read_dir(&fresh)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with(".recording-"))
        .expect("a work directory while recording")
        .path();
    assert_eq!(mode(&work), 0o700, "work directory");
    assert_eq!(mode(&work.join("recording.mp4")), 0o600, "raw video");
    assert_eq!(mode(&work.join("audio.wav")), 0o600, "raw audio");
    rec.sigint();
    let (code, _, stderr) = rec.finish();
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(
        mode(&fresh),
        0o700,
        "captures directory created by comeback"
    );
    let bundle = std::fs::read_dir(&fresh)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "comeback"))
        .expect("a bundle");
    assert_eq!(mode(&bundle), 0o700, "bundle directory");
    assert_eq!(mode(&bundle.join("recording.mp4")), 0o600, "bundled video");
    assert_eq!(mode(&bundle.join("audio.wav")), 0o600, "bundled audio");
    assert_eq!(mode(&bundle.join("frames")), 0o700, "frames directory");

    // A directory that already exists belongs to the user: its mode is left alone.
    let shared = tmp.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut cmd = comeback(Some(&xvfb), &shared);
    cmd.args([
        "record",
        "--area",
        "0,0,320,240",
        "--duration",
        "1",
        "--no-clipboard",
    ]);
    let out = under_open_umask(&cmd).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        mode(&shared),
        0o755,
        "an existing directory is not chmodded"
    );
    let bundle = std::fs::read_dir(&shared)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "comeback"))
        .expect("a bundle in the shared directory");
    assert_eq!(
        mode(&bundle),
        0o700,
        "the bundle inside it is still private"
    );
}
