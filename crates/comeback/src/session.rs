//! One capture from start to bundle. `comeback record` and the daemon both drive this.
//!
//! Raw media lives in `<captures>/.recording-<id>/` until a bundle has been written, so no step
//! after the encoder stops can destroy the only copy of a recording.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use comeback_core::compile::{CompileInput, compile_attached};
use comeback_core::keyframes::KeyframeConfig;
use comeback_core::model::{Target, TargetKind};
use comeback_core::summary;

use crate::gst::{AudioSource, Pipeline, PipelineSpec, Stopped, probe_audio};
use crate::ink::Overlay;
use crate::input::Ink;
use crate::x11::{self, InputReader};
use crate::{clipboard, media, whisper};

const FPS: u32 = 30;
const WORK_PREFIX: &str = ".recording-";

pub struct Options {
    pub captures: PathBuf,
    /// Where the bundle goes. Defaults to `<captures>/<id>.comeback`.
    pub out: Option<PathBuf>,
    pub kind: TargetKind,
    /// x, y, width, height; already fitted to the screen with even sides.
    pub rect: [i32; 4],
    pub app: Option<String>,
    pub title: Option<String>,
    pub clipboard: bool,
}

pub struct Session {
    id: String,
    work: PathBuf,
    out: PathBuf,
    started_secs: u64,
    opts: Options,
    pipeline: Pipeline,
    input: Option<InputReader>,
    overlay: Option<Overlay>,
    notes: Vec<String>,
    audio: Option<PathBuf>,
    video: PathBuf,
}

pub struct Finished {
    pub out: PathBuf,
    pub paste: String,
    /// The tool that took the clipboard text, or why none did. `None` when copying was off.
    pub clipboard: Option<Result<&'static str, String>>,
}

enum AudioChoice {
    None,
    Source(AudioSource),
}

/// `COMEBACK_AUDIO_SOURCE`: unset or `auto` (default microphone), `none`, a PulseAudio source
/// name, or one of three generated sources for machines and tests without a microphone: `test`
/// (a tone), `silence` and `quiet-noise` (white noise far below speech level).
fn audio_choice() -> AudioChoice {
    match std::env::var("COMEBACK_AUDIO_SOURCE").as_deref() {
        Ok("none") => AudioChoice::None,
        Ok("test") => AudioChoice::Source(AudioSource::Test),
        Ok("silence") => AudioChoice::Source(AudioSource::Silence),
        Ok("quiet-noise") => AudioChoice::Source(AudioSource::QuietNoise),
        Ok("auto") | Ok("") | Err(_) => AudioChoice::Source(AudioSource::Pulse(None)),
        Ok(dev) => AudioChoice::Source(AudioSource::Pulse(Some(dev.to_string()))),
    }
}

impl Session {
    pub fn start(opts: Options) -> Result<Self> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let id = format!("comeback-{}", now.as_millis());
        let out = opts
            .out
            .clone()
            .unwrap_or_else(|| opts.captures.join(format!("{id}.comeback")));
        if out.exists() {
            bail!("{} already exists", out.display());
        }
        let work = opts.captures.join(format!("{WORK_PREFIX}{id}"));
        crate::private::create_dir_all(&work)
            .with_context(|| format!("creating {}", work.display()))?;
        match Self::launch(opts, id, out, work.clone(), now.as_secs()) {
            Ok(s) => Ok(s),
            Err(e) => {
                let _ = fs::remove_dir_all(&work);
                Err(e)
            }
        }
    }

    fn launch(opts: Options, id: String, out: PathBuf, work: PathBuf, secs: u64) -> Result<Self> {
        let mut notes = Vec::new();
        let video = work.join("recording.mp4");
        let wav = work.join("audio.wav");
        let audio = match audio_choice() {
            AudioChoice::None => {
                notes.push("recorded video only (audio disabled)".to_string());
                None
            }
            AudioChoice::Source(src) => match probe_audio(&src) {
                Ok(()) => Some(src),
                Err(e) => {
                    notes.push(format!("no microphone, recorded video only ({e})"));
                    None
                }
            },
        };
        let display = std::env::var("DISPLAY").unwrap_or_default();
        // The encoder opens these paths; creating them first makes them private whatever the umask.
        crate::private::create_file(&video)?;
        if audio.is_some() {
            crate::private::create_file(&wav)?;
        }
        let spec = PipelineSpec {
            display,
            rect: opts.rect,
            fps: FPS,
            video: video.clone(),
            audio: audio.map(|a| (a, wav.clone())),
        };
        let has_audio = spec.audio.is_some();
        let pipeline = Pipeline::start(&spec)?;
        let overlay = match Overlay::spawn(opts.rect) {
            Ok(o) => Some(o),
            Err(e) => {
                notes.push(format!("ink overlay unavailable ({e:#})"));
                None
            }
        };
        let tx: Option<std::sync::mpsc::Sender<Ink>> = overlay.as_ref().map(Overlay::sender);
        let input = match InputReader::spawn(opts.rect, pipeline.base, tx) {
            Ok(i) => Some(i),
            Err(e) => {
                notes.push(format!("click and ink capture unavailable ({e:#})"));
                None
            }
        };
        let _ = fs::write(
            work.join("session.json"),
            serde_json::json!({ "id": id, "pid": std::process::id(), "startedAt": iso8601(secs) })
                .to_string(),
        );
        Ok(Self {
            id,
            work,
            out,
            started_secs: secs,
            opts,
            pipeline,
            input,
            overlay,
            notes,
            audio: has_audio.then_some(wav),
            video,
        })
    }

    /// What went wrong or was skipped while starting (no microphone, no ink, ...).
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.pipeline.base.elapsed().as_millis() as u64
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Where the bundle will be written.
    pub fn out_path(&self) -> &Path {
        &self.out
    }

    /// False once the encoder has exited by itself.
    pub fn is_alive(&mut self) -> bool {
        !self.pipeline.has_exited()
    }

    /// Stops capturing, throws the recording away. Only this session's own directory is removed.
    pub fn cancel(mut self) -> Result<()> {
        self.pipeline.kill();
        if let Some(i) = self.input.take() {
            let _ = i.finish();
        }
        if let Some(o) = self.overlay.take() {
            let _ = o.close();
        }
        let name = self
            .work
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if name.is_some_and(|n| n.starts_with(WORK_PREFIX)) {
            fs::remove_dir_all(&self.work)
                .with_context(|| format!("removing {}", self.work.display()))?;
        }
        Ok(())
    }

    /// Stops capturing and compiles the bundle. On failure the raw media stays in
    /// [`work_dir`](Self::work_dir) and the error says where.
    pub fn finish(mut self) -> Result<Finished> {
        let stopped = self.pipeline.stop();
        let captured = match self.input.take() {
            Some(i) => {
                let c = i.finish();
                if let Some(e) = c.error {
                    self.notes.push(format!(
                        "click and ink capture stopped early ({e}); later gestures are missing"
                    ));
                }
                (c.events, c.marks)
            }
            None => (Vec::new(), Vec::new()),
        };
        if let Some(o) = self.overlay.take()
            && let Err(e) = o.close()
        {
            self.notes
                .push(format!("ink overlay stopped early ({e:#})"));
        }
        match stopped {
            Stopped::Clean => {}
            Stopped::Killed => self.notes.push(
                "the encoder did not finish in time, the end of the recording may be missing"
                    .into(),
            ),
            Stopped::Died(code) => self.notes.push(format!(
                "the recording pipeline stopped early (exit {}): {}",
                code.map_or("signal".to_string(), |c| c.to_string()),
                self.pipeline.error_text()
            )),
        }
        let info = match media::probe(&self.video) {
            Ok(i) if i.duration_ms > 0 => i,
            other => {
                let why = other
                    .err()
                    .map_or("the recording is empty".into(), |e| format!("{e:#}"));
                let detail = self.pipeline.error_text();
                // Whatever the encoder wrote, and the audio next to it, stays on disk.
                bail!(
                    "no video was recorded ({why}); pipeline said: {detail}; the raw recording is kept in {}",
                    self.work.display()
                );
            }
        };
        let finished = self.compile(info, captured);
        finished.with_context(|| format!("the raw recording is kept in {}", self.work.display()))
    }

    fn compile(
        &mut self,
        info: media::VideoInfo,
        captured: (
            Vec<comeback_core::model::Event>,
            Vec<comeback_core::model::Mark>,
        ),
    ) -> Result<Finished> {
        let (mut events, mut marks) = captured;
        let duration_ms = info.duration_ms;
        events.retain(|e| e.at_ms <= duration_ms);
        marks.retain(|m| m.at_ms <= duration_ms);

        let mut words = Vec::new();
        let mut transcript_model = None;
        if let Some(wav) = &self.audio {
            let model =
                std::env::var("COMEBACK_WHISPER_MODEL_NAME").unwrap_or_else(|_| "small.en".into());
            match self.transcribe(wav, &model) {
                Ok(Some(w)) => {
                    words = w;
                    transcript_model = Some(model);
                }
                Ok(None) => self
                    .notes
                    .push("no speech detected, transcript left out".into()),
                Err(e) => self.notes.push(format!("transcription failed ({e:#})")),
            }
        }

        let candidates =
            media::candidates(&self.video, &info).context("sampling candidate frames")?;
        let input = CompileInput {
            id: self.id.clone(),
            started_at: iso8601(self.started_secs),
            backend: "x11".into(),
            target: Target {
                kind: self.opts.kind,
                rect: self.opts.rect,
                scale: 1.0,
                app: self.opts.app.clone(),
                title: self.opts.title.clone(),
            },
            duration_ms,
            events,
            marks,
            words,
            transcript_model,
            candidates,
            page: vec![],
            notes: self.notes.clone(),
        };
        let mut attach = vec![self.video.clone()];
        if let Some(wav) = self.audio.as_ref().filter(|w| w.is_file()) {
            attach.push(wav.clone());
        }
        let video = self.video.clone();
        let manifest = compile_attached(
            input,
            &KeyframeConfig::default(),
            |t| media::frame_near(&video, t),
            &self.out,
            &attach,
        )?;
        let _ = fs::remove_dir_all(&self.work);
        let paste = summary::paste_text(&manifest, &self.out);
        let clip = self
            .opts
            .clipboard
            .then(|| clipboard::copy(&paste).map_err(|e| format!("{e:#}")));
        Ok(Finished {
            out: self.out.clone(),
            paste,
            clipboard: clip,
        })
    }

    /// `Ok(None)` when the audio holds no speech.
    fn transcribe(
        &self,
        wav: &Path,
        model: &str,
    ) -> Result<Option<Vec<comeback_core::model::Word>>> {
        let bytes = fs::read(wav).with_context(|| format!("reading {}", wav.display()))?;
        if whisper::is_silent(&whisper::read_samples(&bytes)?) {
            return Ok(None);
        }
        let path = whisper::model_path(model)
            .with_context(|| format!("no ggml-{model}.bin model in ~/.local/share/comeback/models or ~/.local/share/whisper-models"))?;
        whisper::transcribe(wav, &self.work, &path).map(Some)
    }
}

/// The monitor under the pointer, as a ready-to-record rectangle.
pub fn default_screen_rect() -> Result<[i32; 4]> {
    let d = x11::connect()?;
    let screen = x11::screen_rect(&d)?;
    crate::target::fit_area(x11::monitor_under_pointer(&d)?, screen)
}

/// Unix seconds as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_unix_seconds_as_utc_iso() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(951_827_696), "2000-02-29T12:34:56Z");
        assert_eq!(iso8601(1_792_432_139), "2026-10-19T17:48:59Z");
    }
}
