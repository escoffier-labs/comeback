use std::process::Command;

fn comeback() -> Command {
    Command::new(env!("CARGO_BIN_EXE_comeback"))
}

#[test]
fn demo_then_inspect() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("comeback-123.comeback");

    let demo = comeback()
        .args(["demo", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(
        demo.status.success(),
        "{}",
        String::from_utf8_lossy(&demo.stderr)
    );
    let text = String::from_utf8(demo.stdout).unwrap();
    assert!(
        text.starts_with("[comeback 0:20 · demo \"Settings\" · "),
        "{text}"
    );
    assert!(text.contains("[clicked \"New tab\"]"), "{text}");
    assert!(text.contains("[marked]"), "{text}");
    for f in [
        "comeback.json",
        "summary.md",
        "contact-sheet.png",
        "frames/frame-01.png",
    ] {
        assert!(out.join(f).is_file(), "missing {f}");
    }

    let json = comeback()
        .args(["inspect", "--json"])
        .arg(&out)
        .output()
        .unwrap();
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
    let out = comeback()
        .args(["inspect"])
        .env("COMEBACK_CAPTURES_DIR", dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no captures in"));
}

#[test]
fn demo_refuses_an_existing_out() {
    let dir = tempfile::tempdir().unwrap();
    let out = comeback()
        .args(["demo", "--out"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already exists"));
}
