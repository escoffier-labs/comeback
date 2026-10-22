use std::fs;
use std::path::{Path, PathBuf};

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
    frame_at: impl FnMut(u64) -> Result<RgbImage>,
    out: &Path,
) -> Result<Manifest> {
    compile_attached(input, cfg, frame_at, out, &[])
}

/// Like [`compile`], and puts each file in `attach` into the bundle under its own file name.
///
/// Attachments are hard-linked (copied across filesystems), so the originals survive whatever
/// happens to the bundle. The finished bundle is staged next to `out` and swapped in with a
/// rename; an `out` that already exists is only replaced once the new bundle is complete, and is
/// put back if the swap fails.
pub fn compile_attached(
    input: CompileInput,
    cfg: &KeyframeConfig,
    frame_at: impl FnMut(u64) -> Result<RgbImage>,
    out: &Path,
    attach: &[PathBuf],
) -> Result<Manifest> {
    let partial = sibling(out, ".partial")?;
    if partial.exists() {
        fs::remove_dir_all(&partial).with_context(|| format!("removing {}", partial.display()))?;
    }
    private_dir_all(&partial.join("frames"))
        .with_context(|| format!("creating {}", partial.display()))?;
    let built = stage(input, cfg, frame_at, out, attach, &partial)
        .and_then(|manifest| swap_in(&partial, out).map(|()| manifest));
    if built.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    built
}

/// Creates `path` and any missing parents readable by the owner only: a bundle holds the screen
/// and the user's voice, and the process umask must not decide who else can open it. Directories
/// that already exist keep their mode.
fn private_dir_all(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

fn sibling(out: &Path, suffix: &str) -> Result<PathBuf> {
    let mut name = out
        .file_name()
        .with_context(|| format!("{} has no file name", out.display()))?
        .to_os_string();
    name.push(suffix);
    Ok(out.with_file_name(name))
}

fn swap_in(partial: &Path, out: &Path) -> Result<()> {
    if !out.exists() {
        return fs::rename(partial, out)
            .with_context(|| format!("moving bundle to {}", out.display()));
    }
    let old = sibling(out, ".old")?;
    if old.exists() {
        fs::remove_dir_all(&old).with_context(|| format!("removing {}", old.display()))?;
    }
    fs::rename(out, &old).with_context(|| format!("setting aside {}", out.display()))?;
    if let Err(e) = fs::rename(partial, out) {
        let _ = fs::rename(&old, out);
        return Err(e).with_context(|| format!("moving bundle to {}", out.display()));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

fn stage(
    input: CompileInput,
    cfg: &KeyframeConfig,
    mut frame_at: impl FnMut(u64) -> Result<RgbImage>,
    out: &Path,
    attach: &[PathBuf],
    partial: &Path,
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
    for src in attach {
        let name = src
            .file_name()
            .with_context(|| format!("{} has no file name", src.display()))?;
        let dst = partial.join(name);
        if fs::hard_link(src, &dst).is_err() {
            fs::copy(src, &dst).with_context(|| format!("attaching {}", src.display()))?;
        }
    }

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

    fn simple_input(id: &str, clicks: &[u64]) -> CompileInput {
        CompileInput {
            id: id.into(),
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
        }
    }

    fn grey(_: u64) -> Result<RgbImage> {
        Ok(RgbImage::from_pixel(640, 360, Rgb([40, 40, 40])))
    }

    #[test]
    fn attached_media_lands_in_the_bundle_and_the_source_survives() {
        let dir = tempfile::tempdir().unwrap();
        let raw = dir.path().join("raw");
        std::fs::create_dir(&raw).unwrap();
        std::fs::write(raw.join("recording.mp4"), b"video").unwrap();
        std::fs::write(raw.join("audio.wav"), b"audio").unwrap();
        let out = dir.path().join("comeback-4.comeback");
        compile_attached(
            simple_input("comeback-4", &[2_000]),
            &KeyframeConfig::default(),
            grey,
            &out,
            &[raw.join("recording.mp4"), raw.join("audio.wav")],
        )
        .unwrap();
        assert_eq!(std::fs::read(out.join("recording.mp4")).unwrap(), b"video");
        assert_eq!(std::fs::read(out.join("audio.wav")).unwrap(), b"audio");
        assert!(
            raw.join("recording.mp4").is_file(),
            "raw media must survive"
        );
        assert!(raw.join("audio.wav").is_file(), "raw media must survive");
    }

    #[cfg(unix)]
    #[test]
    fn a_bundle_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("comeback-6.comeback");
        compile_attached(
            simple_input("comeback-6", &[2_000]),
            &KeyframeConfig::default(),
            grey,
            &out,
            &[],
        )
        .unwrap();
        for d in [out.clone(), out.join("frames")] {
            let mode = std::fs::metadata(&d).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", d.display());
        }
    }

    #[test]
    fn a_failed_compile_leaves_the_existing_bundle_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("comeback-5.comeback");
        compile(
            simple_input("comeback-5", &[2_000]),
            &KeyframeConfig::default(),
            grey,
            &out,
        )
        .unwrap();
        let before = std::fs::read(out.join("comeback.json")).unwrap();

        let err = compile(
            simple_input("comeback-5", &[2_000, 6_000]),
            &KeyframeConfig::default(),
            |_| anyhow::bail!("decoder exploded"),
            &out,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("decoder exploded"));
        assert_eq!(std::fs::read(out.join("comeback.json")).unwrap(), before);

        let missing = dir.path().join("nope.mp4");
        let err = compile_attached(
            simple_input("comeback-5", &[2_000]),
            &KeyframeConfig::default(),
            grey,
            &out,
            &[missing],
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("nope.mp4"), "{err:#}");
        assert_eq!(std::fs::read(out.join("comeback.json")).unwrap(), before);

        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".partial") || n.ends_with(".old"))
            .collect();
        assert!(strays.is_empty(), "left behind: {strays:?}");
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
