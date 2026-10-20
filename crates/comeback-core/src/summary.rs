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
    writeln!(
        s,
        "[comeback {}{} · {}]",
        fmt_ms(m.duration_ms),
        target,
        dir.display()
    )
    .unwrap();
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
    writeln!(
        s,
        "Contact sheet: {}",
        dir.join("contact-sheet.png").display()
    )
    .unwrap();
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
        assert_eq!(
            got.lines().nth(1),
            Some("Note: transcription failed (no model)")
        );
    }

    #[test]
    fn summary_lists_frame_paths() {
        let got = summary_md(&manifest(), Path::new("/b"));
        assert!(
            got.ends_with("\nFrames:\n- #1 /b/frames/frame-01.png\n- #2 /b/frames/frame-02.png\n")
        );
    }
}
