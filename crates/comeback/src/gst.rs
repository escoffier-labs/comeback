//! The recording pipeline, run as a `gst-launch-1.0` child.
//!
//! One pipeline carries the screen and the microphone so both share a clock. The spec calls for
//! `gstreamer-rs`; this host has the GStreamer runtime but no development headers, so the same
//! pipeline is described as text and launched as a process. Everything outside this module only
//! sees [`Pipeline`].

use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Where audio comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSource {
    /// The default PulseAudio/PipeWire source, or a named one.
    Pulse(Option<String>),
    /// A generated tone, for tests on machines without a microphone.
    Test,
    /// Generated silence, for tests of the no-speech path.
    Silence,
    /// Generated white noise far below speech level, for tests of the noise-floor check.
    QuietNoise,
}

#[derive(Debug, Clone)]
pub struct PipelineSpec {
    pub display: String,
    /// x, y, width, height of the capture; width and height must be even.
    pub rect: [i32; 4],
    pub fps: u32,
    pub video: PathBuf,
    pub audio: Option<(AudioSource, PathBuf)>,
}

/// Arguments for `gst-launch-1.0`: `-e` so SIGINT sends EOS and the files finish, then the pipeline. No `-q`: the
/// "New clock" line it prints when the pipeline starts is how we find time zero.
pub fn launch_args(spec: &PipelineSpec) -> Vec<String> {
    let [x, y, w, h] = spec.rect;
    let fps = spec.fps;
    // Each value is one argv entry: gst-launch escapes spaces inside an entry itself, so a path
    // or device name with spaces or quotes needs no quoting here (and quoting would break it).
    let words = |ws: &[&str]| -> Vec<String> { ws.iter().map(|w| w.to_string()).collect() };
    let mut a = words(&["-e", "ximagesrc"]);
    a.extend([
        format!("display-name={}", spec.display),
        "use-damage=false".into(),
        "show-pointer=true".into(),
        format!("startx={x}"),
        format!("starty={y}"),
        format!("endx={}", x + w - 1),
        format!("endy={}", y + h - 1),
        "!".into(),
        format!("video/x-raw,framerate={fps}/1"),
    ]);
    // Without an explicit format videoconvert hands the RGB screen to x264 as 4:4:4, which
    // browsers, QuickTime and hardware decoders cannot play. I420 gives High profile 4:2:0.
    a.extend(words(&[
        "!",
        "videoconvert",
        "!",
        "video/x-raw,format=I420",
        "!",
        "x264enc",
        "speed-preset=veryfast",
    ]));
    a.extend(words(&["tune=zerolatency", "bitrate=6000"]));
    a.push(format!("key-int-max={fps}"));
    a.extend(words(&["!", "queue", "!", "mux."]));
    if let Some((source, wav)) = &spec.audio {
        match source {
            AudioSource::Pulse(None) => a.push("pulsesrc".into()),
            AudioSource::Pulse(Some(dev)) => {
                a.push("pulsesrc".into());
                a.push(format!("device={dev}"));
            }
            AudioSource::Test => {
                a.extend(words(&[
                    "audiotestsrc",
                    "is-live=true",
                    "wave=sine",
                    "freq=440",
                ]));
            }
            AudioSource::Silence => {
                a.extend(words(&["audiotestsrc", "is-live=true", "wave=silence"]));
            }
            AudioSource::QuietNoise => {
                a.extend(words(&[
                    "audiotestsrc",
                    "is-live=true",
                    "wave=white-noise",
                    "volume=0.002",
                ]));
            }
        }
        a.extend(words(&["!", "audioconvert", "!", "audioresample", "!"]));
        // The WAV is the transcription and silence-check input: pin it to 16-bit PCM instead of
        // whatever the source offers (PulseAudio usually offers 32-bit float). The AAC branch
        // converts back to what the encoder wants.
        a.extend(words(&[
            "audio/x-raw,format=S16LE,rate=16000,channels=1",
            "!",
            "tee",
            "name=t",
        ]));
        a.extend(words(&[
            "t.", "!", "queue", "!", "wavenc", "!", "filesink", UNBUFFERED,
        ]));
        a.push(format!("location={}", wav.display()));
        a.extend(words(&[
            "t.",
            "!",
            "queue",
            "!",
            "audioconvert",
            "!",
            "avenc_aac",
            "!",
            "aacparse",
            "!",
            "queue",
        ]));
        a.extend(words(&["!", "mux."]));
    }
    a.extend(words(&[
        "mp4mux",
        "name=mux",
        "fragment-duration=1000",
        "!",
        "filesink",
        UNBUFFERED,
    ]));
    a.push(format!("location={}", spec.video.display()));
    a
}

/// Written straight to the file, not through stdio's buffer. A killed encoder (OOM, SIGKILL after
/// a stuck EOS) otherwise loses everything still buffered and leaves an unplayable file.
const UNBUFFERED: &str = "buffer-mode=unbuffered";

/// Makes the kernel send `signal` to the child when the process that spawned it dies, even by
/// SIGKILL or the OOM killer, when no destructor runs. The kernel ties this to the spawning
/// *thread*, so spawn from a thread that outlives the child (the main thread).
pub fn die_with_parent(cmd: &mut Command, signal: libc::c_int) {
    let parent = std::process::id();
    // SAFETY: the closure only calls async-signal-safe functions (`prctl`, `getppid`).
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, signal as libc::c_ulong, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl, which the signal then misses.
            if libc::getppid() as u32 != parent {
                return Err(std::io::Error::other("the parent exited during startup"));
            }
            Ok(())
        });
    }
}

/// The `gst-launch-1.0` binary; `COMEBACK_GST_LAUNCH` overrides it (tests use this to fake an
/// encoder that fails).
fn launcher() -> String {
    std::env::var("COMEBACK_GST_LAUNCH").unwrap_or_else(|_| "gst-launch-1.0".into())
}

/// Polls `child` until it exits or `limit` passes.
pub fn wait_timeout(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let end = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if Instant::now() < end => thread::sleep(Duration::from_millis(25)),
            _ => return None,
        }
    }
}

/// Fails unless the audio source can deliver a buffer within a few seconds.
pub fn probe_audio(source: &AudioSource) -> Result<()> {
    let mut probe = vec!["-q".to_string(), "pulsesrc".to_string()];
    match source {
        AudioSource::Test | AudioSource::Silence | AudioSource::QuietNoise => return Ok(()),
        AudioSource::Pulse(None) => {}
        AudioSource::Pulse(Some(dev)) => probe.push(format!("device={dev}")),
    }
    probe.extend(["num-buffers=2", "!", "fakesink"].map(String::from));
    let mut child = Command::new(launcher())
        .args(&probe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", launcher()))?;
    match wait_timeout(&mut child, Duration::from_secs(6)) {
        Some(status) if status.success() => Ok(()),
        Some(_) => {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = e.read_to_string(&mut err);
            }
            bail!("{}", first_line(&err, "the audio source failed to start"))
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the audio source did not produce audio within 6 s")
        }
    }
}

fn first_line(text: &str, fallback: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

/// A running capture pipeline.
pub struct Pipeline {
    child: Child,
    stderr: Arc<Mutex<String>>,
    /// The moment the pipeline reached PLAYING: capture time zero.
    pub base: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Stopped {
    Clean,
    /// EOS did not arrive in time and the encoder was killed; the fragmented MP4 still plays.
    Killed,
    /// The pipeline had already exited with this status before we asked it to stop.
    Died(Option<i32>),
}

impl Pipeline {
    pub fn start(spec: &PipelineSpec) -> Result<Self> {
        let mut cmd = Command::new(launcher());
        cmd.args(launch_args(spec))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Its own process group: a terminal Ctrl-C reaches comeback, which then asks for EOS
            // exactly once, instead of reaching the encoder twice and cutting the file short.
            .process_group(0);
        // If comeback is killed, the encoder gets the same SIGINT a Ctrl-C would send: it
        // writes EOS, finalizes the files and exits, instead of recording on unseen.
        die_with_parent(&mut cmd, libc::SIGINT);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("starting {} (is GStreamer installed?)", launcher()))?;
        let started = Arc::new((Mutex::new(None::<Instant>), Condvar::new()));
        let stdout = child.stdout.take().context("no pipeline stdout")?;
        let flag = started.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                if line.contains("New clock") {
                    *flag.0.lock().expect("start lock") = Some(Instant::now());
                    flag.1.notify_all();
                }
            }
            // Wake the waiter if the pipeline died without ever starting.
            flag.1.notify_all();
        });
        let stderr = Arc::new(Mutex::new(String::new()));
        let err_stream = child.stderr.take().context("no pipeline stderr")?;
        let sink = stderr.clone();
        thread::spawn(move || {
            for line in BufReader::new(err_stream).lines().map_while(|l| l.ok()) {
                let mut s = sink.lock().expect("stderr lock");
                if s.len() < 16_384 {
                    s.push_str(&line);
                    s.push('\n');
                }
            }
        });
        let mut pipeline = Pipeline {
            child,
            stderr,
            base: Instant::now(),
        };
        let deadline = Instant::now() + Duration::from_secs(8);
        let (lock, cv) = &*started;
        let mut guard = lock.lock().expect("start lock");
        while guard.is_none() {
            if let Ok(Some(status)) = pipeline.child.try_wait() {
                bail!(
                    "recording pipeline exited at startup ({status}): {}",
                    pipeline.error_text()
                );
            }
            if Instant::now() >= deadline {
                drop(guard);
                pipeline.kill();
                bail!(
                    "recording pipeline did not start within 8 s: {}",
                    pipeline.error_text()
                );
            }
            guard = cv
                .wait_timeout(guard, Duration::from_millis(50))
                .expect("start lock")
                .0;
        }
        pipeline.base = guard.expect("loop exits once set");
        drop(guard);
        // Source errors (no display, no permission) arrive right after PLAYING.
        if let Some(status) = wait_timeout(&mut pipeline.child, Duration::from_millis(400)) {
            bail!(
                "recording pipeline stopped at startup ({status}): {}",
                pipeline.error_text()
            );
        }
        Ok(pipeline)
    }

    /// The pipeline's own complaint, trimmed for an error message.
    pub fn error_text(&self) -> String {
        let text = self.stderr.lock().expect("stderr lock");
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            return "no error output".into();
        }
        lines[lines.len().saturating_sub(3)..].join(" | ")
    }

    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Sends EOS (SIGINT to `gst-launch -e`) and waits for the files to be finalized.
    pub fn stop(&mut self) -> Stopped {
        if let Ok(Some(status)) = self.child.try_wait() {
            return Stopped::Died(status.code());
        }
        let _ = Command::new("kill")
            .args(["-INT", &self.child.id().to_string()])
            .stderr(Stdio::null())
            .status();
        match wait_timeout(&mut self.child, Duration::from_secs(10)) {
            Some(_) => Stopped::Clean,
            None => {
                self.kill();
                Stopped::Killed
            }
        }
    }

    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            self.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(audio: Option<(AudioSource, PathBuf)>) -> PipelineSpec {
        PipelineSpec {
            display: ":77".into(),
            rect: [10, 20, 640, 360],
            fps: 30,
            video: "/tmp/w/recording.mp4".into(),
            audio,
        }
    }

    #[test]
    fn video_branch_covers_the_exact_region() {
        let a = launch_args(&spec(None)).join(" ");
        assert!(a.starts_with("-e ximagesrc display-name=:77"), "{a}");
        assert!(a.contains("startx=10 starty=20 endx=649 endy=379"), "{a}");
        assert!(a.contains("use-damage=false"), "{a}");
        assert!(a.contains("video/x-raw,framerate=30/1"), "{a}");
        assert!(
            a.contains("videoconvert ! video/x-raw,format=I420 ! x264enc"),
            "browser-playable 4:2:0: {a}"
        );
        assert!(
            a.contains("fragment-duration=1000 ! filesink buffer-mode=unbuffered"),
            "a killed encoder keeps what it wrote: {a}"
        );
        assert!(a.contains("fragment-duration=1000"), "{a}");
        assert!(a.contains("location=/tmp/w/recording.mp4"), "{a}");
    }

    #[test]
    fn audio_branch_exists_only_when_requested() {
        let none = launch_args(&spec(None)).join(" ");
        assert!(
            !none.contains("wavenc") && !none.contains("avenc_aac"),
            "{none}"
        );
        let with = launch_args(&spec(Some((
            AudioSource::Pulse(None),
            "/tmp/w/audio.wav".into(),
        ))))
        .join(" ");
        assert!(with.contains("pulsesrc"), "{with}");
        assert!(
            with.contains("audio/x-raw,format=S16LE,rate=16000,channels=1"),
            "the WAV is pinned to 16-bit PCM: {with}"
        );
        assert!(
            with.contains("queue ! audioconvert ! avenc_aac"),
            "the AAC branch converts back for the encoder: {with}"
        );
        assert!(with.contains("wavenc"), "{with}");
        assert!(with.contains("avenc_aac"), "{with}");
        assert!(with.contains("location=/tmp/w/audio.wav"), "{with}");
        let tone = launch_args(&spec(Some((AudioSource::Test, "/tmp/a.wav".into())))).join(" ");
        assert!(tone.contains("audiotestsrc"), "{tone}");
    }

    #[test]
    fn silence_uses_the_generator_with_no_tone() {
        let a = launch_args(&spec(Some((AudioSource::Silence, "/tmp/a.wav".into())))).join(" ");
        assert!(a.contains("audiotestsrc is-live=true wave=silence"), "{a}");
    }

    #[test]
    fn named_sources_and_odd_paths_stay_single_arguments() {
        let s = PipelineSpec {
            video: "/tmp/we ird\"dir/rec.mp4".into(),
            ..spec(Some((
                AudioSource::Pulse(Some("alsa_input.usb mic".into())),
                "/tmp/a b.wav".into(),
            )))
        };
        let args = launch_args(&s);
        // gst-launch escapes spaces inside one argv entry, so values must not be split or quoted.
        for want in [
            "location=/tmp/we ird\"dir/rec.mp4",
            "device=alsa_input.usb mic",
            "location=/tmp/a b.wav",
        ] {
            assert!(
                args.iter().any(|a| a == want),
                "{want:?} not an argument in {args:?}"
            );
        }
    }
}
