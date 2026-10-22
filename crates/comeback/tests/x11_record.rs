//! End-to-end recording tests on a throwaway Xvfb display. They never touch the user's own
//! `DISPLAY`. When Xvfb, GStreamer or ffmpeg are missing the tests skip with a message, unless
//! `COMEBACK_REQUIRE_X11_TESTS=1` (CI) turns that into a failure.

mod common;

use std::thread;
use std::time::Duration;

use common::*;

#[test]
fn records_real_video_audio_clicks_and_ink() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("real.comeback");
    let model = tmp.path().join("ggml-small.en.bin");
    std::fs::write(&model, b"not a real model").unwrap();

    let mut cmd = comeback(Some(&xvfb), &captures);
    cmd.args([
        "record",
        "--area",
        "0,0,640,360",
        "--duration",
        "7",
        "--no-clipboard",
        "--out",
    ])
    .arg(&bundle)
    .env("COMEBACK_WHISPER_CLI", fake_whisper(tmp.path()))
    .env("COMEBACK_WHISPER_MODEL", &model);
    let mut rec = Recorder::spawn(cmd);
    rec.wait_for("comeback: recording 640x360");
    thread::sleep(Duration::from_millis(1500));

    user.click(320, 180);
    user.paint(GREEN);
    thread::sleep(Duration::from_millis(900));
    user.ink((100, 100), (300, 220));

    let (code, stdout, stderr) = rec.finish();
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.starts_with("[comeback 0:0"), "{stdout}");

    for f in [
        "comeback.json",
        "summary.md",
        "contact-sheet.png",
        "frames/frame-01.png",
        "recording.mp4",
        "audio.wav",
    ] {
        assert!(bundle.join(f).is_file(), "missing {f}");
    }
    assert!(
        leftovers(&captures).is_empty() && leftovers(tmp.path()).is_empty(),
        "leftovers: {:?}",
        leftovers(&captures)
    );

    if let Some(keep) = std::env::var_os("COMEBACK_TEST_KEEP") {
        let _ = std::process::Command::new("cp")
            .arg("-r")
            .arg(&bundle)
            .arg(keep)
            .status();
    }
    let streams = probe_streams(&bundle.join("recording.mp4"));
    assert!(streams.contains(&"h264,640,360".to_string()), "{streams:?}");
    assert!(
        streams.iter().any(|s| s.starts_with("aac")),
        "microphone audio is muxed: {streams:?}"
    );
    assert_eq!(
        video_format(&bundle.join("recording.mp4")),
        "High,yuv420p",
        "browsers and QuickTime need 4:2:0, not 4:4:4"
    );
    assert_eq!(
        audio_format(&bundle.join("audio.wav")),
        "pcm_s16le,16000,1",
        "the WAV is 16-bit mono PCM"
    );
    let rms = wav_rms(&bundle.join("audio.wav"));
    assert!(rms > 3_000.0, "the tone is really in the WAV: rms {rms}");
    let wav = std::fs::read(bundle.join("audio.wav")).unwrap();
    assert!(
        wav.len() > 16_000 * 2 * 4,
        "about 7 s of 16 kHz mono PCM, got {} bytes",
        wav.len()
    );

    let m = manifest(&bundle);
    assert_eq!(m["schema"], 1);
    assert_eq!(m["backend"], "x11");
    assert_eq!(m["target"]["kind"], "area");
    assert_eq!(m["target"]["rect"], serde_json::json!([0, 0, 640, 360]));
    let click = m["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "click")
        .expect("a click event");
    assert!(
        (click["x"].as_f64().unwrap() - 0.5).abs() < 0.02
            && (click["y"].as_f64().unwrap() - 0.5).abs() < 0.02,
        "{click}"
    );
    assert_eq!(click["button"], "left");
    let click_ms = click["atMs"].as_u64().unwrap();

    // The video and the input events share one clock: the screen turns green right after the click.
    let green_ms = first_green_ms(&bundle.join("recording.mp4"));
    eprintln!("sync: click at {click_ms} ms, screen green at {green_ms} ms");
    assert!(
        green_ms.abs_diff(click_ms) <= 350,
        "click at {click_ms} ms, screen green at {green_ms} ms"
    );

    let video = bundle.join("recording.mp4");
    assert!(
        near(
            mean(&frame_png(&video, 0.5), (500, 20, 600, 80)),
            BLUE,
            30.0
        ),
        "start of the video is blue"
    );
    let end = m["durationMs"].as_f64().unwrap() / 1000.0 - 0.4;
    assert!(
        near(
            mean(&frame_png(&video, end), (500, 20, 600, 80)),
            GREEN,
            30.0
        ),
        "end of the video is green"
    );

    let marks = m["marks"].as_array().unwrap();
    assert!(!marks.is_empty(), "the Shift-drag became a mark: {m}");
    let n = marks[0]["keyframe"]
        .as_u64()
        .expect("the mark owns a keyframe") as usize;
    let kf = image::open(bundle.join(format!("frames/frame-{n:02}.png")))
        .unwrap()
        .to_rgb8();
    let red = kf
        .pixels()
        .filter(|p| p[0] > 170 && p[1] < 100 && p[2] < 100)
        .count();
    eprintln!("ink: {red} red pixels in keyframe {n}");
    assert!(red > 150, "red ink pixels in the mark keyframe: {red}");

    assert_eq!(m["transcript"]["model"], "small.en");
    assert_eq!(m["transcript"]["words"].as_array().unwrap().len(), 5);
    assert!(
        m["keyframes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["said"].as_str().unwrap().contains("Click here now")),
        "{m}"
    );
    assert!(
        m.get("notes").is_none(),
        "no notes expected: {}",
        m["notes"]
    );
    assert!(
        stdout.contains("[clicked") && stdout.contains("[marked]"),
        "{stdout}"
    );
}

#[test]
fn no_display_is_a_clear_error() {
    let tmp = tempfile::tempdir().unwrap();
    let out = comeback(None, tmp.path())
        .args(["record", "--duration", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("DISPLAY is not set"));
    assert!(leftovers(tmp.path()).is_empty());
}

#[test]
fn unreachable_display_is_a_clear_error() {
    let tmp = tempfile::tempdir().unwrap();
    let out = comeback(None, tmp.path())
        .env("DISPLAY", ":9871")
        .args(["record", "--duration", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot open X11 display :9871"));
}

#[test]
fn invalid_areas_are_rejected_before_anything_starts() {
    let tmp = tempfile::tempdir().unwrap();
    for (area, want) in [("10,10,5", "X,Y,W,H"), ("0,0,0,10", "positive")] {
        let out = comeback(None, tmp.path())
            .args(["record", "--area", area])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{area}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(want), "{area}: {err}");
    }
    let out = comeback(None, tmp.path())
        .args(["record", "--duration", "0"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn areas_outside_the_screen_are_rejected() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let tmp = tempfile::tempdir().unwrap();
    for (area, want) in [
        ("0,0,5000,5000", "outside the screen"),
        ("1200,0,400,300", "outside the screen"),
        ("0,0,8,8", "at least 16"),
    ] {
        let out = comeback(Some(&xvfb), tmp.path())
            .args(["record", "--area", area])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{area}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(want),
            "{area}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert!(leftovers(tmp.path()).is_empty());
}

#[test]
fn an_existing_output_is_left_alone() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let existing = tmp.path().join("mine.comeback");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("keep.txt"), "precious").unwrap();
    let out = comeback(Some(&xvfb), &captures)
        .args([
            "record",
            "--area",
            "0,0,320,240",
            "--duration",
            "1",
            "--out",
        ])
        .arg(&existing)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(existing.join("keep.txt")).unwrap(),
        "precious"
    );
    assert!(leftovers(&captures).is_empty());
}

#[test]
fn an_encoder_that_fails_leaves_nothing_behind() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let fake = tmp.path().join("broken-gst");
    std::fs::write(&fake, "#!/bin/sh\necho 'ERROR: from element /GstPipeline:pipeline0/GstX264Enc: no encoder' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = comeback(Some(&xvfb), &captures)
        .env("COMEBACK_GST_LAUNCH", &fake)
        .env("COMEBACK_AUDIO_SOURCE", "none")
        .args(["record", "--area", "0,0,320,240", "--duration", "1"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("recording pipeline") && err.contains("no encoder"),
        "{err}"
    );
    assert!(
        leftovers(&captures).is_empty(),
        "{:?}",
        leftovers(&captures)
    );
    let bundles = std::fs::read_dir(&captures).map(|d| d.count()).unwrap_or(0);
    assert_eq!(bundles, 0, "no bundle for a failed capture");
}

#[test]
fn a_missing_microphone_records_video_only_with_a_note() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("quiet.comeback");
    let out = comeback(Some(&xvfb), &captures)
        .env_remove("COMEBACK_AUDIO_SOURCE")
        // A PulseAudio server that does not exist: the default microphone cannot open.
        .env("PULSE_SERVER", "unix:/nonexistent/comeback-test-socket")
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
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!bundle.join("audio.wav").exists());
    let streams = probe_streams(&bundle.join("recording.mp4"));
    assert!(streams.iter().all(|s| !s.starts_with("aac")), "{streams:?}");
    let m = manifest(&bundle);
    assert!(m["transcript"].is_null());
    let note = m["notes"][0]
        .as_str()
        .expect("a note explains the missing audio");
    assert!(
        note.contains("no microphone, recorded video only"),
        "{note}"
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("Note: no microphone, recorded video only"),
        "{text}"
    );
    assert!(leftovers(&captures).is_empty());
}

#[test]
fn stopping_early_still_makes_a_bundle() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("short.comeback");
    let mut cmd = comeback(Some(&xvfb), &captures);
    cmd.args(["record", "--area", "0,0,320,240", "--no-clipboard", "--out"])
        .arg(&bundle);
    let mut rec = Recorder::spawn(cmd);
    rec.wait_for("comeback: recording");
    thread::sleep(Duration::from_millis(1200));
    rec.sigint();
    let (code, stdout, stderr) = rec.finish();
    assert_eq!(code, Some(0), "stderr: {stderr}");
    assert!(stdout.starts_with("[comeback 0:0"), "{stdout}");
    let m = manifest(&bundle);
    let ms = m["durationMs"].as_u64().unwrap();
    assert!((500..4000).contains(&ms), "duration {ms} ms");
    assert!(bundle.join("recording.mp4").is_file());
    assert!(leftovers(&captures).is_empty());
}

#[test]
fn a_failure_after_capture_keeps_the_raw_media() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("never.comeback");
    // ffmpeg exists for the live capture but frame decoding breaks afterwards.
    let broken = tmp.path().join("broken-ffmpeg");
    std::fs::write(&broken, "#!/bin/sh\necho 'decoder exploded' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&broken, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = comeback(Some(&xvfb), &captures)
        .env("COMEBACK_FFMPEG", &broken)
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
    assert!(err.contains("the raw recording is kept in"), "{err}");
    assert!(!bundle.exists(), "no half-written bundle");
    let kept: Vec<_> = std::fs::read_dir(&captures)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".recording-"))
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(
        kept[0]
            .path()
            .join("recording.mp4")
            .metadata()
            .unwrap()
            .len()
            > 1000
    );
    assert!(kept[0].path().join("audio.wav").is_file());
}

#[test]
fn silence_and_a_missing_model_become_notes_not_failures() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");

    // silent microphone: nothing to transcribe
    let quiet = tmp.path().join("quiet.comeback");
    let out = comeback(Some(&xvfb), &captures)
        .env("COMEBACK_AUDIO_SOURCE", "silence")
        .args([
            "record",
            "--area",
            "0,0,320,240",
            "--duration",
            "2",
            "--no-clipboard",
            "--out",
        ])
        .arg(&quiet)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let m = manifest(&quiet);
    assert!(m["transcript"].is_null());
    assert!(
        m["notes"][0]
            .as_str()
            .unwrap()
            .contains("no speech detected"),
        "{m}"
    );
    assert!(
        quiet.join("audio.wav").is_file(),
        "the silent audio is still kept"
    );

    // quiet noise, as a real room has: well above digital silence, far below speech
    let hiss = tmp.path().join("hiss.comeback");
    let out = comeback(Some(&xvfb), &captures)
        .env("COMEBACK_AUDIO_SOURCE", "quiet-noise")
        .env("COMEBACK_WHISPER_CLI", "/nonexistent/whisper must not run")
        .args([
            "record",
            "--area",
            "0,0,320,240",
            "--duration",
            "2",
            "--no-clipboard",
            "--out",
        ])
        .arg(&hiss)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(audio_format(&hiss.join("audio.wav")), "pcm_s16le,16000,1");
    let rms = wav_rms(&hiss.join("audio.wav"));
    assert!(
        (5.0..100.0).contains(&rms),
        "the WAV holds real quiet noise, not zeros and not loud garbage: rms {rms}"
    );
    let m = manifest(&hiss);
    assert!(m["transcript"].is_null());
    assert!(
        m["notes"][0]
            .as_str()
            .unwrap()
            .contains("no speech detected"),
        "quiet noise is not speech, and whisper never ran: {m}"
    );

    // speech but no model installed: the bundle is still made
    let nomodel = tmp.path().join("nomodel.comeback");
    let empty_home = tmp.path().join("home");
    std::fs::create_dir_all(&empty_home).unwrap();
    let out = comeback(Some(&xvfb), &captures)
        .env("HOME", &empty_home)
        .env("XDG_DATA_HOME", empty_home.join("share"))
        .env_remove("COMEBACK_WHISPER_MODEL")
        .args([
            "record",
            "--area",
            "0,0,320,240",
            "--duration",
            "2",
            "--no-clipboard",
            "--out",
        ])
        .arg(&nomodel)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let m = manifest(&nomodel);
    assert!(m["transcript"].is_null());
    let note = m["notes"][0].as_str().unwrap();
    assert!(
        note.starts_with("transcription failed (no ggml-small.en.bin"),
        "{note}"
    );
    assert!(nomodel.join("recording.mp4").is_file() && nomodel.join("audio.wav").is_file());
    assert!(leftovers(&captures).is_empty());
}

#[test]
fn window_mode_records_the_clicked_window() {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConnectionExt as _, CreateWindowAux, PropMode, WindowClass,
    };
    use x11rb::wrapper::ConnectionExt as _;
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    let user = xvfb.client();
    user.paint(BLUE);
    // A 400x300 window at (200, 120) with a class and title, as an application would set.
    let win = user.conn.generate_id().unwrap();
    user.conn
        .create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            user.root,
            200,
            120,
            400,
            300,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new().background_pixel(0x00_c0_a0_20),
        )
        .unwrap();
    user.conn
        .change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"Settings",
        )
        .unwrap();
    user.conn
        .change_property8(
            PropMode::REPLACE,
            win,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"firefox\0Firefox\0",
        )
        .unwrap();
    user.conn.map_window(win).unwrap();
    user.conn.flush().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let bundle = tmp.path().join("win.comeback");
    let mut cmd = comeback(Some(&xvfb), &captures);
    cmd.args([
        "record",
        "--window",
        "--duration",
        "3",
        "--no-clipboard",
        "--out",
    ])
    .arg(&bundle);
    let mut rec = Recorder::spawn(cmd);
    rec.wait_for("click the window to record");
    thread::sleep(Duration::from_millis(500));
    user.click(400, 250);
    let (code, stdout, stderr) = rec.finish();
    assert_eq!(code, Some(0), "stderr: {stderr}");
    let m = manifest(&bundle);
    assert_eq!(m["target"]["kind"], "window");
    assert_eq!(m["target"]["rect"], serde_json::json!([200, 120, 400, 300]));
    assert_eq!(m["target"]["app"], "firefox");
    assert_eq!(m["target"]["title"], "Settings");
    assert!(stdout.contains("firefox \"Settings\""), "{stdout}");
    let streams = probe_streams(&bundle.join("recording.mp4"));
    assert!(streams.contains(&"h264,400,300".to_string()), "{streams:?}");
    // the picking click itself is not part of the recording
    assert!(
        m["events"].as_array().unwrap().is_empty(),
        "{}",
        m["events"]
    );
}

#[test]
fn the_encoder_stops_when_the_recorder_is_killed() {
    let Some(_guard) = prerequisites() else {
        return;
    };
    let xvfb = Xvfb::start();
    xvfb.client().paint(BLUE);
    let tmp = tempfile::tempdir().unwrap();
    let captures = tmp.path().join("captures");
    let mut cmd = comeback(Some(&xvfb), &captures);
    cmd.args(["record", "--area", "0,0,320,240", "--no-clipboard"]);
    let mut rec = Recorder::spawn(cmd);
    rec.wait_for("comeback: recording");
    thread::sleep(Duration::from_millis(1500));

    let needle = format!("{}/.recording-", captures.display());
    let encoders = pids_matching(&needle);
    assert!(!encoders.is_empty(), "an encoder is running");
    // SIGKILL runs no destructor, exactly like the OOM killer.
    rec.child.kill().unwrap();
    rec.child.wait().unwrap();

    let end = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < end && !pids_matching(&needle).is_empty() {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        pids_matching(&needle).is_empty(),
        "the encoder {encoders:?} outlived its killed recorder"
    );
    // It finished the file on its way out instead of leaving a stub.
    let work = std::fs::read_dir(&captures)
        .unwrap()
        .flatten()
        .find(|e| e.file_name().to_string_lossy().starts_with(".recording-"))
        .expect("the raw recording is still on disk")
        .path();
    let secs = media_seconds(&work.join("recording.mp4"));
    assert!(secs > 0.8, "playable video survives: {secs} s");
    assert!(work.join("audio.wav").metadata().unwrap().len() > 10_000);
}
