use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use image::RgbImage;

use crate::keyframes::{Candidate, KeyframeConfig, SelectInput, select};
use crate::model::{
    Event, Keyframe, Manifest, Mark, PageEvent, Reason, SCHEMA, Target, Transcript, Word,
};
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
    let mut partial_name = out
        .file_name()
        .with_context(|| format!("{} has no file name", out.display()))?
        .to_os_string();
    partial_name.push(".partial");
    let partial = out.with_file_name(partial_name);
    if partial.exists() {
        fs::remove_dir_all(&partial).with_context(|| format!("removing {}", partial.display()))?;
    }
    fs::create_dir_all(partial.join("frames"))
        .with_context(|| format!("creating {}", partial.display()))?;

    let mut marks = input.marks;
    let mut frames = Vec::with_capacity(selected.len());
    let mut keyframes = Vec::with_capacity(selected.len());
    for (i, s) in selected.iter().enumerate() {
        let n = i as u32 + 1;
        let mut img = frame_at(s.at_ms).with_context(|| format!("frame at {} ms", s.at_ms))?;
        if let (Reason::Click, Some([x, y])) = (s.reason, s.point) {
            draw_ripple(&mut img, x, y);
        }
        draw_badge(&mut img, n, s.at_ms);
        let file = format!("frames/frame-{n:02}.png");
        img.save(partial.join(&file))
            .with_context(|| format!("writing {file}"))?;
        for m in marks.iter_mut().filter(|m| s.mark_ids.contains(&m.id)) {
            m.keyframe = Some(n);
        }
        keyframes.push(Keyframe {
            n,
            at_ms: s.at_ms,
            reason: s.reason,
            file,
            said: s.said.clone(),
            action: s.action.clone(),
        });
        frames.push(img);
    }
    render_sheet(&frames)
        .save(partial.join("contact-sheet.png"))
        .context("writing contact-sheet.png")?;

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
        transcript: input.transcript_model.map(|model| Transcript {
            model,
            words: input.words,
        }),
        notes: input.notes,
    };
    fs::write(
        partial.join("comeback.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )
    .context("writing comeback.json")?;
    fs::write(partial.join("summary.md"), summary_md(&manifest, out))
        .context("writing summary.md")?;
    if out.exists() {
        fs::remove_dir_all(out).with_context(|| format!("removing {}", out.display()))?;
    }
    fs::rename(&partial, out).with_context(|| format!("moving bundle to {}", out.display()))?;
    Ok(manifest)
}

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
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 640, 360],
                scale: 1.0,
                app: None,
                title: None,
            },
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
        let m = compile(
            input,
            &KeyframeConfig::default(),
            |_| Ok(RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]))),
            &out,
        )
        .unwrap();
        assert_eq!(m.keyframes.len(), 2);
        assert_eq!(m.marks[0].keyframe, Some(2));
        assert!(m.transcript.is_none());
        for f in [
            "comeback.json",
            "summary.md",
            "contact-sheet.png",
            "frames/frame-01.png",
            "frames/frame-02.png",
        ] {
            assert!(out.join(f).is_file(), "missing {f}");
        }
        let back: Manifest =
            serde_json::from_slice(&std::fs::read(out.join("comeback.json")).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn drags_get_no_ripple() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("comeback-2.comeback");
        let input = CompileInput {
            id: "comeback-2".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            backend: "test".into(),
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 640, 360],
                scale: 1.0,
                app: None,
                title: None,
            },
            duration_ms: 10_000,
            events: vec![Event {
                at_ms: 2_000,
                kind: EventKind::Drag,
                x: 0.5,
                y: 0.5,
                to: Some([0.9, 0.5]),
                button: None,
                label: None,
            }],
            marks: vec![],
            words: vec![],
            transcript_model: None,
            candidates: vec![],
            page: vec![],
            notes: vec![],
        };
        compile(
            input,
            &KeyframeConfig::default(),
            |_| Ok(RgbImage::from_pixel(640, 360, Rgb([40, 40, 40]))),
            &out,
        )
        .unwrap();
        let frame = image::open(out.join("frames/frame-01.png"))
            .unwrap()
            .to_rgb8();
        assert_eq!(*frame.get_pixel(330, 180), Rgb([40, 40, 40]));
    }

    #[test]
    fn recompiling_replaces_the_bundle_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("comeback-3.comeback");
        let make = |clicks: &[u64]| CompileInput {
            id: "comeback-3".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            backend: "test".into(),
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 640, 360],
                scale: 1.0,
                app: None,
                title: None,
            },
            duration_ms: 10_000,
            events: clicks
                .iter()
                .map(|&at_ms| Event {
                    at_ms,
                    kind: EventKind::Click,
                    x: 0.5,
                    y: 0.5,
                    to: None,
                    button: None,
                    label: None,
                })
                .collect(),
            marks: vec![],
            words: vec![],
            transcript_model: None,
            candidates: vec![],
            page: vec![],
            notes: vec![],
        };
        let frame = |_| Ok(RgbImage::from_pixel(640, 360, Rgb([40, 40, 40])));
        compile(
            make(&[2_000, 6_000]),
            &KeyframeConfig::default(),
            frame,
            &out,
        )
        .unwrap();
        assert!(out.join("frames/frame-02.png").is_file());
        compile(make(&[2_000]), &KeyframeConfig::default(), frame, &out).unwrap();
        assert!(out.join("frames/frame-01.png").is_file());
        assert!(!out.join("frames/frame-02.png").exists());
        let partials = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".partial"))
            .count();
        assert_eq!(partials, 0);
    }
}
