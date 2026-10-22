//! `comeback daemon`, `toggle`, `cancel` and `status`.
//!
//! The daemon owns at most one [`Session`]. Alt+Shift+S (an `XGrabKey` on the root window) and
//! the control socket both feed one message queue, so a hotkey press and `comeback toggle` mean
//! exactly the same thing. The socket is mode 0600 inside a directory nobody else can enter.
//!
//! Stopping hands the finished capture to a worker thread that compiles the bundle (Whisper
//! and a full decode can take minutes). The daemon keeps answering meanwhile: `status` says
//! `compiling`, `cancel` says it cannot undo a finished capture, and toggles are refused as
//! busy rather than queued, so no press made during a compile starts a recording afterwards.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use comeback_core::model::TargetKind;
use x11rb::connection::Connection;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::xkb::{BoolCtrl, ConnectionExt as _, ID as XKB_ID, PerClientFlag};
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};

use crate::session::{Options, Session, default_screen_rect};
use crate::x11;

/// A second toggle this soon after a start is a double press, not a request to stop.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// A hotkey press that sat in the queue this long is not the user's intent any more.
const STALE: Duration = Duration::from_secs(2);
/// `s`
const KEYSYM_S: u32 = 0x73;

static QUIT: AtomicBool = AtomicBool::new(false);

type Reply = Option<Sender<String>>;

enum Msg {
    /// With the moment it was queued: a hotkey press behind a slow start is stale.
    Toggle(Reply, Instant),
    Cancel(Reply),
    Status(Reply),
}

/// What the daemon is doing.
enum State {
    Idle,
    Recording(Box<Session>, Instant),
    /// A stopped capture is being turned into a bundle on a worker thread. Nothing can undo it.
    Compiling {
        id: String,
        out: PathBuf,
        since: Instant,
        done: Receiver<Outcome>,
    },
}

/// A worker's result: the text for the stopper, and who is waiting for it.
struct Outcome {
    text: String,
    reply: Reply,
}

/// `$COMEBACK_SOCKET`, else `$XDG_RUNTIME_DIR/comeback.sock`.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("COMEBACK_SOCKET") {
        return Ok(PathBuf::from(p));
    }
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .ok_or_else(|| {
            anyhow!("XDG_RUNTIME_DIR is not set; set COMEBACK_SOCKET to choose a socket path")
        })?;
    Ok(PathBuf::from(dir).join("comeback.sock"))
}

fn notify(text: &str) {
    if std::env::var("COMEBACK_NOTIFY").as_deref() == Ok("off") {
        return;
    }
    let text = text.to_string();
    thread::spawn(move || {
        let _ = Command::new("notify-send")
            .args(["-a", "comeback", "-t", "3000", "comeback", &text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

/// Refuses a socket directory other users could enter, so the socket's own mode is not the only guard.
fn check_private_dir(dir: &Path) -> Result<()> {
    let meta = fs::metadata(dir).with_context(|| format!("checking {}", dir.display()))?;
    if meta.mode() & 0o077 != 0 {
        bail!(
            "{} is accessible to other users (mode {:o}); keep the control socket in a private directory",
            dir.display(),
            meta.mode() & 0o777
        );
    }
    Ok(())
}

/// Clears a leftover socket from a crashed daemon. A live daemon, or a file that is not a socket, is an error.
fn claim_socket_path(path: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("checking {}", path.display())),
    };
    if !meta.file_type().is_socket() {
        bail!(
            "{} exists and is not a socket; refusing to remove it",
            path.display()
        );
    }
    if UnixStream::connect(path).is_ok() {
        bail!("a comeback daemon is already running ({})", path.display());
    }
    fs::remove_file(path).with_context(|| format!("removing stale socket {}", path.display()))
}

pub fn run(captures: PathBuf) -> Result<()> {
    let display = x11::connect().context("the daemon needs an X11 session")?;
    drop(display);
    let socket = socket_path()?;
    let dir = socket
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    check_private_dir(dir)?;
    claim_socket_path(&socket)?;
    let listener =
        UnixListener::bind(&socket).with_context(|| format!("binding {}", socket.display()))?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    ctrlc::set_handler(|| QUIT.store(true, Ordering::SeqCst))
        .context("installing the signal handler")?;

    let (tx, rx) = mpsc::channel::<Msg>();
    let accept_tx = tx.clone();
    thread::Builder::new()
        .name("comeback-socket".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = accept_tx.clone();
                thread::spawn(move || serve(stream, tx));
            }
        })?;
    let hotkey_note = match spawn_hotkey(tx) {
        Ok(()) => "Alt+Shift+S grabbed".to_string(),
        Err(e) => format!("no hotkey ({e:#}); use `comeback toggle`"),
    };
    eprintln!(
        "comeback: daemon ready (socket {}, {hotkey_note})",
        socket.display()
    );

    let result = serve_messages(&rx, &captures);
    let _ = fs::remove_file(&socket);
    result
}

fn serve_messages(rx: &mpsc::Receiver<Msg>, captures: &Path) -> Result<()> {
    let mut state = State::Idle;
    let say = |reply: Reply, text: String| {
        if let Some(r) = reply {
            let _ = r.send(text);
        }
    };
    loop {
        if let State::Compiling { done, .. } = &state {
            match done.try_recv() {
                Ok(outcome) => {
                    report(outcome, &say);
                    state = State::Idle;
                }
                Err(TryRecvError::Disconnected) => {
                    eprintln!("comeback: the compile thread died; the raw recording was kept");
                    state = State::Idle;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if QUIT.load(Ordering::SeqCst) {
            match std::mem::replace(&mut state, State::Idle) {
                // A recording in progress is finished, not thrown away, when the daemon stops.
                State::Recording(s, _) => eprintln!("comeback: {}", finish(*s)),
                // So is one already being compiled: wait for it.
                State::Compiling { done, .. } => {
                    if let Ok(outcome) = done.recv() {
                        report(outcome, &say);
                    }
                }
                State::Idle => {}
            }
            return Ok(());
        }
        let msg = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => {
                let dead = match &mut state {
                    State::Recording(s, _) => !s.is_alive(),
                    _ => false,
                };
                if dead {
                    let State::Recording(s, _) = std::mem::replace(&mut state, State::Idle) else {
                        unreachable!("matched above");
                    };
                    notify("Recording stopped unexpectedly, saving what was captured");
                    state = compile(*s, None);
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        match msg {
            Msg::Status(r) => say(
                r,
                match &mut state {
                    State::Idle => "ok idle".into(),
                    State::Recording(s, _) => {
                        format!("ok recording {} ({} ms)", s.id(), s.elapsed_ms())
                    }
                    State::Compiling { id, since, .. } => {
                        format!("ok compiling {id} ({} ms)", since.elapsed().as_millis())
                    }
                },
            ),
            Msg::Cancel(r) => match std::mem::replace(&mut state, State::Idle) {
                State::Idle => say(r, "ok nothing to cancel".into()),
                State::Recording(s, _) => match s.cancel() {
                    Ok(()) => {
                        notify("Recording discarded");
                        say(r, "ok cancelled".into());
                    }
                    Err(e) => say(r, one_line(format!("error {e:#}"))),
                },
                busy @ State::Compiling { .. } => {
                    let State::Compiling { id, out, .. } = &busy else {
                        unreachable!("matched above");
                    };
                    say(
                        r,
                        one_line(format!(
                            "error {id} is already recorded and is being saved to {}; nothing was cancelled and the capture is kept",
                            out.display()
                        )),
                    );
                    state = busy;
                }
            },
            Msg::Toggle(r, queued) => match std::mem::replace(&mut state, State::Idle) {
                State::Idle if r.is_none() && queued.elapsed() > STALE => {
                    eprintln!(
                        "comeback: dropped a hotkey press that waited {:?}",
                        queued.elapsed()
                    );
                }
                State::Idle => match start(captures) {
                    Ok(s) => {
                        for note in s.notes() {
                            eprintln!("comeback: note: {note}");
                        }
                        notify(&match s.notes().first() {
                            Some(note) => format!("Recording ({note})"),
                            None => "Recording".to_string(),
                        });
                        say(r, format!("ok recording {}", s.id()));
                        state = State::Recording(Box::new(s), Instant::now());
                    }
                    Err(e) => {
                        notify(&format!("Could not start: {e:#}"));
                        say(r, one_line(format!("error {e:#}")));
                    }
                },
                State::Recording(s, at) if at.elapsed() < DEBOUNCE => {
                    state = State::Recording(s, at);
                    say(r, "ok ignored (debounce)".into());
                }
                State::Recording(s, _) => state = compile(*s, r),
                busy @ State::Compiling { .. } => {
                    let State::Compiling { id, since, .. } = &busy else {
                        unreachable!("matched above");
                    };
                    notify("Still saving the last recording");
                    say(
                        r,
                        one_line(format!(
                            "error busy: {id} is still being saved ({} s so far); this toggle was ignored, try again when `comeback status` says idle",
                            since.elapsed().as_secs()
                        )),
                    );
                    state = busy;
                }
            },
        }
    }
}

/// Tells the stopper, and the desktop, how a compile went.
fn report(outcome: Outcome, say: &impl Fn(Reply, String)) {
    let ok = !outcome.text.starts_with("error");
    notify(if ok { "Copied" } else { "Recording failed" });
    if !ok {
        eprintln!("comeback: {}", outcome.text);
    }
    say(
        outcome.reply,
        one_line(if ok {
            format!("ok {}", outcome.text)
        } else {
            outcome.text
        }),
    );
}

/// Stops `s` and compiles its bundle on a worker thread, so the daemon stays responsive.
/// `reply` gets the result when it is ready.
fn compile(s: Session, reply: Reply) -> State {
    let (id, out) = (s.id().to_string(), s.out_path().to_path_buf());
    let (tx, done) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("comeback-compile".into())
        .spawn(move || {
            let _ = tx.send(Outcome {
                text: finish(s),
                reply,
            });
        });
    if let Err(e) = spawned {
        // The session went down with the closure: its destructors stopped the encoder.
        eprintln!("comeback: cannot start the compile thread: {e}");
        return State::Idle;
    }
    State::Compiling {
        id,
        out,
        since: Instant::now(),
        done,
    }
}

fn start(captures: &Path) -> Result<Session> {
    Session::start(Options {
        captures: captures.to_path_buf(),
        out: None,
        kind: TargetKind::Screen,
        rect: default_screen_rect()?,
        app: None,
        title: None,
        clipboard: true,
    })
}

/// `copied <path>`, `saved <path> (clipboard: ...)` or `error ...`.
fn finish(s: Session) -> String {
    match s.finish() {
        Ok(f) => match f.clipboard {
            Some(Ok(_)) | None => format!("copied {}", f.out.display()),
            Some(Err(e)) => format!("saved {} (clipboard: {e})", f.out.display()),
        },
        Err(e) => format!("error {e:#}"),
    }
}

fn one_line(s: String) -> String {
    s.replace(['\n', '\r'], " ")
}

/// One request per connection: a verb line in, one reply line out.
fn serve(stream: UnixStream, tx: Sender<Msg>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    // A bounded read: nothing legitimate is longer than a verb.
    if BufReader::new((&stream).take(64))
        .read_line(&mut line)
        .is_err()
    {
        return;
    }
    let (rtx, rrx) = mpsc::channel();
    let msg = match line.trim() {
        "toggle" => Msg::Toggle(Some(rtx), Instant::now()),
        "cancel" => Msg::Cancel(Some(rtx)),
        "status" => Msg::Status(Some(rtx)),
        other => {
            let _ = writeln!(
                &stream,
                "error unknown command {:?}",
                other.chars().take(20).collect::<String>()
            );
            return;
        }
    };
    if tx.send(msg).is_err() {
        return;
    }
    // Stopping compiles a bundle, which can take a while on a long capture.
    let reply = rrx
        .recv_timeout(Duration::from_secs(900))
        .unwrap_or_else(|_| "error daemon did not answer".into());
    let _ = writeln!(&stream, "{reply}");
}

/// Grabs Alt+Shift+S (with and without NumLock and CapsLock) on its own X connection.
fn spawn_hotkey(tx: Sender<Msg>) -> Result<()> {
    let d = x11::connect()?;
    // A held key must be one KeyPress and one KeyRelease. Without this the server sends a
    // release and a press for every autorepeat, and each press would toggle again.
    d.conn.xkb_use_extension(1, 0)?.reply()?;
    let flag = PerClientFlag::DETECTABLE_AUTO_REPEAT;
    let flags = d
        .conn
        .xkb_per_client_flags(
            XKB_ID::USE_CORE_KBD.into(),
            flag,
            flag,
            BoolCtrl::from(0u16),
            BoolCtrl::from(0u16),
            BoolCtrl::from(0u16),
        )?
        .reply()
        .context("cannot ask the X server to report autorepeat as repeats")?;
    if u32::from(flags.supported) & u32::from(flag) == 0 {
        bail!("the X server cannot report autorepeat, so a held hotkey would toggle repeatedly");
    }
    let setup = d.conn.setup();
    let (min, max) = (setup.min_keycode, setup.max_keycode);
    let map = d.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
    let per = usize::from(map.keysyms_per_keycode);
    let keycode = map
        .keysyms
        .chunks(per)
        .position(|syms| syms.contains(&KEYSYM_S))
        .map(|i| min + i as u8)
        .context("no key produces `s` on this keyboard")?;
    let base = ModMask::M1 | ModMask::SHIFT;
    for extra in [
        ModMask::from(0u16),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::LOCK | ModMask::M2,
    ] {
        d.conn
            .grab_key(
                true,
                d.root,
                base | extra,
                keycode,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .check()
            .context("Alt+Shift+S is already grabbed by another program")?;
    }
    d.conn.flush()?;
    thread::Builder::new()
        .name("comeback-hotkey".into())
        .spawn(move || {
            let mut pressed = Press::default();
            while let Ok(event) = d.conn.wait_for_event() {
                let toggle = match event {
                    XEvent::KeyPress(e) if e.detail == keycode => pressed.down(Instant::now()),
                    XEvent::KeyRelease(e) if e.detail == keycode => {
                        pressed.up();
                        false
                    }
                    _ => false,
                };
                if toggle && tx.send(Msg::Toggle(None, Instant::now())).is_err() {
                    break;
                }
            }
        })?;
    Ok(())
}

/// Turns a stream of KeyPress/KeyRelease into one toggle per physical press.
#[derive(Default)]
struct Press {
    /// When the key was last seen going down or repeating, while it is held.
    held_since: Option<Instant>,
}

impl Press {
    /// A repeat of a key that is still down toggles nothing. A key that has been silent for
    /// longer than any autorepeat gap and shows a press again must have lost its release
    /// (the keyboard was grabbed elsewhere, say), so that is a new press.
    fn down(&mut self, now: Instant) -> bool {
        const REPEAT_GAP: Duration = Duration::from_millis(1500);
        let repeat = self
            .held_since
            .is_some_and(|t| now.saturating_duration_since(t) < REPEAT_GAP);
        self.held_since = Some(now);
        !repeat
    }

    fn up(&mut self) {
        self.held_since = None;
    }
}

/// `comeback toggle|cancel|status`: forwards one verb to the daemon and prints its answer.
pub fn client(verb: &str) -> Result<()> {
    let socket = socket_path()?;
    let mut stream = UnixStream::connect(&socket).map_err(|e| {
        anyhow!("the comeback daemon is not running (no answer on {}: {e}). Start it with `comeback daemon`", socket.display())
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(900)))?;
    writeln!(stream, "{verb}")?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    let reply = reply.trim();
    println!("{reply}");
    if reply.starts_with("ok") {
        Ok(())
    } else {
        bail!("{}", reply.strip_prefix("error ").unwrap_or(reply))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_key_toggles_once_however_often_it_repeats() {
        let t0 = Instant::now();
        let mut key = Press::default();
        assert!(key.down(t0), "the physical press toggles");
        for i in 1..=40 {
            let repeat = t0 + Duration::from_millis(660 + 40 * i);
            assert!(!key.down(repeat), "autorepeat {i} is not a new press");
        }
        key.up();
        assert!(
            key.down(t0 + Duration::from_secs(3)),
            "the next press toggles"
        );
    }

    #[test]
    fn a_press_after_a_lost_release_still_counts() {
        let t0 = Instant::now();
        let mut key = Press::default();
        assert!(key.down(t0));
        assert!(
            key.down(t0 + Duration::from_secs(5)),
            "no repeats for 5 s and no release: the release was lost"
        );
    }
}
