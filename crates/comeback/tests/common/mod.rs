//! Shared helpers for the Xvfb tests: a throwaway X server, a fake user, and a process wrapper.
#![allow(dead_code)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux, EventMask, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// One Xvfb at a time: each recording runs an encoder, and this host is easily overloaded.
pub static SERIAL: Mutex<()> = Mutex::new(());

pub const BLUE: (u8, u8, u8) = (0x20, 0x40, 0xa0);
pub const GREEN: (u8, u8, u8) = (0x30, 0xa0, 0x40);

pub fn have(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

/// `None` (after printing why) when this machine cannot run the X11 tests.
pub fn prerequisites() -> Option<MutexGuard<'static, ()>> {
    let missing: Vec<&str> = [
        ("Xvfb", on_path("Xvfb")),
        ("gst-launch-1.0", have("gst-launch-1.0", &["--version"])),
        ("ximagesrc", have("gst-inspect-1.0", &["ximagesrc"])),
        ("x264enc", have("gst-inspect-1.0", &["x264enc"])),
        ("avenc_aac", have("gst-inspect-1.0", &["avenc_aac"])),
        ("ffmpeg", have("ffmpeg", &["-version"])),
        ("ffprobe", have("ffprobe", &["-version"])),
    ]
    .into_iter()
    .filter(|(_, ok)| !ok)
    .map(|(n, _)| n)
    .collect();
    if !missing.is_empty() {
        if std::env::var("COMEBACK_REQUIRE_X11_TESTS").as_deref() == Ok("1") {
            panic!("X11 test prerequisites missing: {missing:?}");
        }
        eprintln!("SKIP: X11 test prerequisites missing: {missing:?}");
        return None;
    }
    Some(SERIAL.lock().unwrap_or_else(|e| e.into_inner()))
}

pub struct Xvfb {
    child: Child,
    pub display: String,
}

impl Xvfb {
    pub fn start() -> Self {
        let mut child = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1280x720x24",
                "-nolisten",
                "tcp",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start Xvfb");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .expect("Xvfb display number");
        let display = format!(":{}", line.trim());
        Xvfb { child, display }
    }

    pub fn client(&self) -> Client {
        let (conn, screen) = x11rb::connect(Some(&self.display)).expect("connect to Xvfb");
        let root = conn.setup().roots[screen].root;
        Client {
            conn,
            root,
            at: std::cell::Cell::new(None),
        }
    }
}

impl Xvfb {
    /// Takes the display away while clients are still connected.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Plays the user: paints the screen and injects pointer and key input through XTest.
pub struct Client {
    pub conn: RustConnection,
    pub root: u32,
    /// Where the fake pointer was last put. XTest delivers a motion event even for a move to
    /// the spot it is already on, which a real hand does not, so such moves are skipped.
    at: std::cell::Cell<Option<(i16, i16)>>,
}

impl Client {
    pub fn paint(&self, (r, g, b): (u8, u8, u8)) {
        let px = (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
        self.conn
            .change_window_attributes(
                self.root,
                &ChangeWindowAttributesAux::new().background_pixel(px),
            )
            .unwrap();
        self.conn.clear_area(false, self.root, 0, 0, 0, 0).unwrap();
        self.conn.flush().unwrap();
    }

    pub fn fake(&self, kind: u8, detail: u8, x: i16, y: i16) {
        self.conn
            .xtest_fake_input(kind, detail, 0, self.root, x, y, 0)
            .unwrap();
        self.conn.flush().unwrap();
    }

    pub fn move_to(&self, x: i16, y: i16) {
        if self.at.get() == Some((x, y)) {
            return;
        }
        self.at.set(Some((x, y)));
        self.fake(6, 0, x, y);
    }
    pub fn button(&self, down: bool, button: u8) {
        self.fake(if down { 4 } else { 5 }, button, 0, 0);
    }
    pub fn shift(&self, down: bool) {
        self.fake(if down { 2 } else { 3 }, 50, 0, 0);
    }
    /// Shift-click: Shift down, press and release without moving, Shift up.
    pub fn shift_click(&self, x: i16, y: i16) {
        self.move_to(x, y);
        thread::sleep(Duration::from_millis(60));
        self.shift(true);
        thread::sleep(Duration::from_millis(60));
        self.button(true, 1);
        thread::sleep(Duration::from_millis(80));
        self.button(false, 1);
        thread::sleep(Duration::from_millis(60));
        self.shift(false);
    }
    /// A plain left-button drag along a diagonal, no modifiers.
    pub fn drag(&self, (x0, y0): (i16, i16), (x1, y1): (i16, i16)) {
        self.move_to(x0, y0);
        thread::sleep(Duration::from_millis(60));
        self.button(true, 1);
        for i in 1..=10 {
            let f = f64::from(i) / 10.0;
            self.move_to(
                (f64::from(x0) + f * f64::from(x1 - x0)) as i16,
                (f64::from(y0) + f * f64::from(y1 - y0)) as i16,
            );
            thread::sleep(Duration::from_millis(40));
        }
        self.button(false, 1);
        thread::sleep(Duration::from_millis(60));
    }
    /// The root window's pixel at x, y as (r, g, b).
    pub fn pixel(&self, x: i16, y: i16) -> (u8, u8, u8) {
        use x11rb::protocol::xproto::ImageFormat;
        let img = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, x, y, 1, 1, !0)
            .unwrap()
            .reply()
            .unwrap();
        (img.data[2], img.data[1], img.data[0])
    }
    pub fn click(&self, x: i16, y: i16) {
        self.move_to(x, y);
        thread::sleep(Duration::from_millis(50));
        self.button(true, 1);
        thread::sleep(Duration::from_millis(60));
        self.button(false, 1);
    }
    /// Shift-drag a diagonal stroke from (x0, y0) to (x1, y1).
    pub fn ink(&self, (x0, y0): (i16, i16), (x1, y1): (i16, i16)) {
        self.shift(true);
        thread::sleep(Duration::from_millis(60));
        self.move_to(x0, y0);
        thread::sleep(Duration::from_millis(60));
        self.button(true, 1);
        for i in 1..=14 {
            let f = f64::from(i) / 14.0;
            let x = f64::from(x0) + f * f64::from(x1 - x0);
            let y = f64::from(y0) + f * f64::from(y1 - y0);
            self.move_to(x as i16, y as i16);
            thread::sleep(Duration::from_millis(40));
        }
        self.button(false, 1);
        thread::sleep(Duration::from_millis(60));
        self.shift(false);
    }
}

pub struct Recorder {
    pub child: Child,
    lines: mpsc::Receiver<String>,
    pub seen: Vec<String>,
}

pub fn comeback(xvfb: Option<&Xvfb>, captures: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_comeback"));
    c.env_remove("WAYLAND_DISPLAY")
        .env("COMEBACK_CAPTURES_DIR", captures)
        .env("COMEBACK_AUDIO_SOURCE", "test")
        .stdin(Stdio::null());
    match xvfb {
        Some(x) => c.env("DISPLAY", &x.display),
        None => c.env_remove("DISPLAY"),
    };
    c
}

impl Recorder {
    pub fn spawn(mut cmd: Command) -> Self {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run comeback");
        let (tx, lines) = mpsc::channel();
        let err = child.stderr.take().unwrap();
        thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(|l| l.ok()) {
                let _ = tx.send(l);
            }
        });
        Recorder {
            child,
            lines,
            seen: Vec::new(),
        }
    }

    /// Blocks until stderr shows a line containing `needle`.
    pub fn wait_for(&mut self, needle: &str) {
        let end = Instant::now() + Duration::from_secs(30);
        while Instant::now() < end {
            if let Ok(l) = self.lines.recv_timeout(Duration::from_millis(100)) {
                let hit = l.contains(needle);
                self.seen.push(l);
                if hit {
                    return;
                }
            }
        }
        panic!("never saw {needle:?}; stderr so far: {:?}", self.seen);
    }

    pub fn sigint(&self) {
        Command::new("kill")
            .args(["-INT", &self.child.id().to_string()])
            .status()
            .unwrap();
    }

    /// (exit code, stdout, all stderr)
    pub fn finish(mut self) -> (Option<i32>, String, String) {
        let out = self.child.stdout.take().map(|mut s| {
            let mut b = String::new();
            std::io::Read::read_to_string(&mut s, &mut b).ok();
            b
        });
        let status = self.child.wait().unwrap();
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(200)) {
            self.seen.push(l);
        }
        (status.code(), out.unwrap_or_default(), self.seen.join("\n"))
    }
}

pub fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| {
                    n.starts_with(".recording-") || n.ends_with(".partial") || n.ends_with(".old")
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn manifest(bundle: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(bundle.join("comeback.json")).unwrap()).unwrap()
}

pub fn frame_png(video: &Path, at: f64) -> image::RgbImage {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &format!("{at:.3}"), "-i"])
        .arg(video)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .unwrap();
    image::load_from_memory(&out.stdout)
        .expect("decodable frame")
        .to_rgb8()
}

/// Mean colour of a region.
pub fn mean(img: &image::RgbImage, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> (f64, f64, f64) {
    let (mut r, mut g, mut b, mut n) = (0.0, 0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y);
            r += f64::from(p[0]);
            g += f64::from(p[1]);
            b += f64::from(p[2]);
            n += 1.0;
        }
    }
    (r / n, g / n, b / n)
}

pub fn near(got: (f64, f64, f64), want: (u8, u8, u8), tol: f64) -> bool {
    (got.0 - f64::from(want.0)).abs() <= tol
        && (got.1 - f64::from(want.1)).abs() <= tol
        && (got.2 - f64::from(want.2)).abs() <= tol
}

/// Milliseconds into the video at which the picture first turns green-dominant.
pub fn first_green_ms(video: &Path) -> u64 {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(video)
        .args([
            "-vf",
            "fps=30,scale=64:36",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    const FRAME: usize = 64 * 36 * 3;
    for (i, f) in out.stdout.as_chunks::<FRAME>().0.iter().enumerate() {
        let (mut g, mut b) = (0u64, 0u64);
        for px in f.as_chunks::<3>().0 {
            g += u64::from(px[1]);
            b += u64::from(px[2]);
        }
        if g > b {
            return i as u64 * 1000 / 30;
        }
    }
    panic!("the video never turns green");
}

pub fn probe_streams(video: &Path) -> Vec<String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// `profile,pix_fmt` of the first video stream.
pub fn video_format(video: &Path) -> String {
    ffprobe_csv(video, "v:0", "stream=pix_fmt,profile")
}

/// `codec_name,sample_rate,channels` of the first audio stream.
pub fn audio_format(audio: &Path) -> String {
    ffprobe_csv(audio, "a:0", "stream=codec_name,sample_rate,channels")
}

fn ffprobe_csv(file: &Path, select: &str, entries: &str) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", select])
        .args(["-show_entries", entries, "-of", "csv=p=0"])
        .arg(file)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Seconds of media, per ffprobe.
pub fn media_seconds(file: &Path) -> f64 {
    ffprobe_csv(file, "v:0", "format=duration")
        .parse()
        .unwrap_or(0.0)
}

/// RMS of an audio file on the 16-bit scale, decoded by ffmpeg whatever the file's own format.
pub fn wav_rms(wav: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(wav)
        .args(["-f", "s16le", "-ac", "1", "-ar", "16000", "-"])
        .output()
        .unwrap();
    let samples: Vec<f64> = out
        .stdout
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f64::from(i16::from_le_bytes(*b)))
        .collect();
    assert!(
        !samples.is_empty(),
        "no audio decoded from {}",
        wav.display()
    );
    (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt()
}

/// PIDs whose command line contains `needle`.
pub fn pids_matching(needle: &str) -> Vec<u32> {
    let out = Command::new("pgrep")
        .args(["-f", "--", needle])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// The same command, run by a shell whose umask is 000 (new files would be world-writable).
pub fn under_open_umask(cmd: &Command) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg("umask 000; exec \"$0\" \"$@\"")
        .arg(cmd.get_program())
        .args(cmd.get_args())
        .stdin(Stdio::null());
    for (k, v) in cmd.get_envs() {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
    c
}

pub fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Like [`fake_whisper`], but it takes `secs` seconds first: a transcription that is slow enough
/// to ask the daemon things while it works.
pub fn slow_whisper(dir: &Path, secs: u32) -> PathBuf {
    let fast = fake_whisper(dir);
    let path = dir.join("slow-whisper");
    std::fs::write(
        &path,
        format!("#!/bin/sh\nsleep {secs}\nexec {} \"$@\"\n", fast.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A stand-in for `whisper-cli` that writes canned word-level JSON where `-of` points.
pub fn fake_whisper(dir: &Path) -> PathBuf {
    let path = dir.join("fake-whisper");
    std::fs::write(
        &path,
        r#"#!/bin/sh
while [ $# -gt 0 ]; do
  case "$1" in -of) prefix="$2"; shift;; esac
  shift
done
cat > "$prefix.json" <<'EOF'
{"transcription":[
 {"offsets":{"from":0,"to":300},"text":"","tokens":[]},
 {"offsets":{"from":1900,"to":2100},"text":" Click","tokens":[]},
 {"offsets":{"from":2100,"to":2300},"text":" here","tokens":[]},
 {"offsets":{"from":2300,"to":2500},"text":" now.","tokens":[]},
 {"offsets":{"from":3400,"to":3700},"text":" Wrong","tokens":[]},
 {"offsets":{"from":3700,"to":4000},"text":" label.","tokens":[]}
]}
EOF
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// What a client window was told by the X server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    Press(u8),
    Release(u8),
    Motion,
}

/// A mapped application window that selects button and motion events, so every press on it
/// starts the server's implicit pointer grab, exactly like a real application. It records what
/// it receives, which is how the tests tell "the app saw the click" from "comeback swallowed it".
pub struct AppWindow {
    pub rect: (i16, i16, u16, u16),
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl AppWindow {
    pub fn open(xvfb: &Xvfb, rect: (i16, i16, u16, u16)) -> Self {
        let (conn, screen) = x11rb::connect(Some(&xvfb.display)).expect("connect app");
        let root = conn.setup().roots[screen].root;
        let win = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            win,
            root,
            rect.0,
            rect.1,
            rect.2,
            rect.3,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(0x00_f0_f0_f0)
                .event_mask(
                    EventMask::BUTTON_PRESS
                        | EventMask::BUTTON_RELEASE
                        | EventMask::POINTER_MOTION
                        | EventMask::BUTTON_MOTION,
                ),
        )
        .unwrap();
        conn.map_window(win).unwrap();
        conn.flush().unwrap();
        // Round trip: the window is mapped before any test input is injected.
        conn.get_input_focus().unwrap().reply().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, flag) = (seen.clone(), stop.clone());
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::SeqCst) {
                match conn.poll_for_event() {
                    Ok(Some(XEvent::ButtonPress(e))) => {
                        log.lock().unwrap().push(Seen::Press(e.detail))
                    }
                    Ok(Some(XEvent::ButtonRelease(e))) => {
                        log.lock().unwrap().push(Seen::Release(e.detail))
                    }
                    Ok(Some(XEvent::MotionNotify(_))) => log.lock().unwrap().push(Seen::Motion),
                    Ok(Some(_)) => {}
                    Ok(None) => thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        });
        AppWindow {
            rect,
            seen,
            stop,
            thread: Some(thread),
        }
    }

    /// A point `dx`, `dy` inside the window, in screen coordinates.
    pub fn at(&self, dx: i16, dy: i16) -> (i16, i16) {
        (self.rect.0 + dx, self.rect.1 + dy)
    }

    pub fn seen(&self) -> Vec<Seen> {
        thread::sleep(Duration::from_millis(150));
        self.seen.lock().unwrap().clone()
    }

    pub fn presses(&self) -> usize {
        self.seen()
            .iter()
            .filter(|s| matches!(s, Seen::Press(_)))
            .count()
    }

    pub fn clear(&self) {
        thread::sleep(Duration::from_millis(150));
        self.seen.lock().unwrap().clear();
    }
}

impl Drop for AppWindow {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
