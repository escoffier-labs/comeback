//! X11 plumbing: connection, screen geometry, window picking and the raw pointer reader.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use comeback_core::model::{Event, Mark};
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::Event as XEvent;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xinput::{self, ConnectionExt as _, XIEventMask};
use x11rb::protocol::xproto::{
    Allow, AtomEnum, ButtonIndex, ConnectionExt as _, EventMask, GrabMode, GrabStatus, KeyButMask,
    ModMask, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::input::{Gestures, Ink, Raw};

/// `XIAllMasterDevices`: raw events from the virtual core pointer and keyboard.
const ALL_MASTER_DEVICES: xinput::DeviceId = 1;
/// How often the pointer is sampled while a button is held.
const MOTION_POLL: Duration = Duration::from_millis(10);

pub struct Display {
    pub conn: RustConnection,
    pub root: Window,
}

/// Connects to `$DISPLAY`. Fails with a plain message when there is no X server to talk to.
pub fn connect() -> Result<Display> {
    let name = std::env::var("DISPLAY")
        .ok()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| anyhow!("no X11 display: DISPLAY is not set"))?;
    let (conn, screen) =
        x11rb::connect(Some(&name)).with_context(|| format!("cannot open X11 display {name}"))?;
    let root = conn.setup().roots[screen].root;
    Ok(Display { conn, root })
}

/// The whole screen: x, y, width, height.
pub fn screen_rect(d: &Display) -> Result<[i32; 4]> {
    let g = d.conn.get_geometry(d.root)?.reply()?;
    Ok([0, 0, i32::from(g.width), i32::from(g.height)])
}

/// The RandR monitor containing the pointer, or the whole screen when RandR has none.
pub fn monitor_under_pointer(d: &Display) -> Result<[i32; 4]> {
    let p = d.conn.query_pointer(d.root)?.reply()?;
    let (px, py) = (i32::from(p.root_x), i32::from(p.root_y));
    if let Ok(reply) = d.conn.randr_get_monitors(d.root, true)?.reply() {
        for m in &reply.monitors {
            let (x, y) = (i32::from(m.x), i32::from(m.y));
            let (w, h) = (i32::from(m.width), i32::from(m.height));
            if px >= x && py >= y && px < x + w && py < y + h && w > 0 && h > 0 {
                return Ok([x, y, w, h]);
            }
        }
    }
    screen_rect(d)
}

pub struct Picked {
    pub rect: [i32; 4],
    pub app: Option<String>,
    pub title: Option<String>,
}

/// Waits for one click and returns the top-level window under it.
pub fn pick_window(d: &Display) -> Result<Picked> {
    let grab = d
        .conn
        .grab_pointer(
            false,
            d.root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            CURRENT_TIME,
        )?
        .reply()?;
    if grab.status != GrabStatus::SUCCESS {
        bail!("cannot pick a window: the pointer is grabbed by another program");
    }
    let released = || {
        let _ = d.conn.ungrab_pointer(CURRENT_TIME);
        let _ = d.conn.flush();
    };
    let child = loop {
        match d.conn.wait_for_event()? {
            XEvent::ButtonPress(e) => break e.child,
            _ => continue,
        }
    };
    released();
    if child == x11rb::NONE {
        bail!("no window under the pointer (clicked the desktop background)");
    }
    let g = d.conn.get_geometry(child)?.reply()?;
    let bw = i32::from(g.border_width);
    let (w, h) = (i32::from(g.width) + 2 * bw, i32::from(g.height) + 2 * bw);
    let (app, title) = describe(d, child);
    Ok(Picked {
        rect: [i32::from(g.x), i32::from(g.y), w, h],
        app,
        title,
    })
}

/// Class and title of a top-level window, looking one level into a window manager frame.
fn describe(d: &Display, win: Window) -> (Option<String>, Option<String>) {
    let mut candidates = vec![win];
    if let Ok(Ok(tree)) = d.conn.query_tree(win).map(|c| c.reply()) {
        candidates.extend(tree.children);
    }
    let utf8 = intern(d, "UTF8_STRING");
    let net_name = intern(d, "_NET_WM_NAME");
    let text = |w: Window, prop: u32, ty: u32| -> Option<Vec<u8>> {
        let r = d
            .conn
            .get_property(false, w, prop, ty, 0, 256)
            .ok()?
            .reply()
            .ok()?;
        (!r.value.is_empty()).then_some(r.value)
    };
    let mut app = None;
    let mut title = None;
    for w in candidates {
        if title.is_none() {
            let t = text(w, net_name, utf8)
                .or_else(|| text(w, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()));
            title = t.map(|v| String::from_utf8_lossy(&v).into_owned());
        }
        if app.is_none() {
            // WM_CLASS is "instance\0class\0"; the class is the readable name.
            app = text(w, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into()).and_then(|v| {
                let parts: Vec<_> = v.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
                parts
                    .last()
                    .map(|p| String::from_utf8_lossy(p).to_lowercase())
            });
        }
    }
    (app, title)
}

fn intern(d: &Display, name: &str) -> u32 {
    d.conn
        .intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|c| c.reply().ok())
        .map_or(0, |r| r.atom)
}

/// Reads raw pointer events on its own thread and turns them into the manifest timeline.
///
/// It also owns Shift-left-drag. Drawing ink must not click, select or drag in the application
/// underneath, so a passive grab on the root window catches Shift+Button1 before any
/// application sees it. The grab is synchronous: the server freezes the pointer at the press
/// (raw events and pointer queries included) until the press is dealt with, which decides:
///
/// - a press inside the recorded target is drawing: `AsyncPointer` thaws the pointer with the
///   grab active, so the whole stroke goes to comeback and the application never hears of it;
/// - a press anywhere else is the application's: `ReplayPointer` hands it over as if the grab
///   never existed, so Shift-clicks and Shift-drags outside the recording work as usual.
///
/// The decision has to be made at the press: nothing moves while the pointer is frozen, and a
/// replay is only possible for the press that triggered the grab.
pub struct InputReader {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Captured>,
}

/// What the input thread collected. An X error ends the capture but keeps every gesture
/// recorded before it: `error` says why it stopped early.
pub struct Captured {
    pub events: Vec<Event>,
    pub marks: Vec<Mark>,
    pub error: Option<String>,
}

/// XInput 2.2 is the first version that delivers raw events while a pointer grab is active.
/// With 2.0 or 2.1 a press on an ordinary window (an implicit grab) hides its release from us.
const XI_MAJOR: u16 = 2;
const XI_MINOR: u16 = 2;

/// Fails with an explanation when the server's XInput is too old for raw events through grabs.
fn require_xi(major: u16, minor: u16) -> Result<()> {
    if (major, minor) < (XI_MAJOR, XI_MINOR) {
        bail!(
            "this X server offers XInput {major}.{minor} but recording needs {XI_MAJOR}.{XI_MINOR}: older versions hide button releases while any window holds a pointer grab, which would lose clicks"
        );
    }
    Ok(())
}

/// Shift is the only modifier that draws. NumLock and CapsLock do not count, so the grab is
/// installed for each combination of them.
fn lock_combos() -> [ModMask; 4] {
    [
        ModMask::from(0u16),
        ModMask::LOCK,
        ModMask::M2,
        ModMask::LOCK | ModMask::M2,
    ]
}

/// Modifier bits that make a press something other than a plain Shift press.
fn other_modifiers() -> KeyButMask {
    KeyButMask::CONTROL | KeyButMask::MOD1 | KeyButMask::MOD3 | KeyButMask::MOD4 | KeyButMask::MOD5
}

/// What the synchronous Shift+Button1 grab is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GrabState {
    /// No Shift press is in the grab.
    Idle,
    /// The press is ours (drawing). The grab is active until the button is released.
    Owned,
}

/// The connection, the Shift+Button1 grab on the root window, and its state. Dropping it gives
/// the pointer back whatever happened: a stop, an X error or a panic. A crash closes the
/// connection, which makes the server release every grab too.
struct Grabs {
    d: Display,
    state: GrabState,
}

impl Grabs {
    /// Installs the grab. Another program already holding Shift+Button1 is an error.
    fn install(d: Display) -> Result<Self> {
        let grabs = Self {
            d,
            state: GrabState::Idle,
        };
        for extra in lock_combos() {
            let cookie = grabs.d.conn.grab_button(
                false,
                grabs.d.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::SYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                ButtonIndex::M1,
                ModMask::SHIFT | extra,
            )?;
            cookie.check().context(
                "Shift+left-drag is already grabbed by another program, so ink cannot be drawn",
            )?;
        }
        grabs.d.conn.flush()?;
        Ok(grabs)
    }

    /// Decides the frozen press: hands it to the application (`replay`) or keeps it (drawing).
    fn settle(&mut self, replay: bool) -> Result<()> {
        let mode = if replay {
            Allow::REPLAY_POINTER
        } else {
            Allow::ASYNC_POINTER
        };
        self.d.conn.allow_events(mode, CURRENT_TIME)?;
        self.d.conn.flush()?;
        self.state = if replay {
            GrabState::Idle
        } else {
            GrabState::Owned
        };
        Ok(())
    }
}

impl Drop for Grabs {
    fn drop(&mut self) {
        let c = &self.d.conn;
        // A stroke still being drawn, or a press frozen in the server, is let go either way.
        if self.state == GrabState::Owned {
            let _ = c.ungrab_pointer(CURRENT_TIME);
        }
        let _ = c.allow_events(Allow::REPLAY_POINTER, CURRENT_TIME);
        for extra in lock_combos() {
            let _ = c.ungrab_button(ButtonIndex::M1, self.d.root, ModMask::SHIFT | extra);
        }
        let _ = c.flush();
    }
}

impl InputReader {
    /// Connects and subscribes on the calling thread so setup errors surface to the caller.
    /// `base` is the capture clock origin; ink updates are sent to `ink` as they happen.
    pub fn spawn(target: [i32; 4], base: Instant, ink: Option<Sender<Ink>>) -> Result<Self> {
        let d = connect()?;
        let v = d
            .conn
            .xinput_xi_query_version(XI_MAJOR, XI_MINOR)?
            .reply()
            .context("XInput 2 is not available on this X server")?;
        require_xi(v.major_version, v.minor_version)?;
        let mask = xinput::EventMask {
            deviceid: ALL_MASTER_DEVICES,
            // Motion is polled while a button is held: warped pointers (remote desktop, xdotool)
            // produce no raw motion events, and idle motion is not worth waking up for.
            mask: vec![XIEventMask::RAW_BUTTON_PRESS | XIEventMask::RAW_BUTTON_RELEASE],
        };
        d.conn.xinput_xi_select_events(d.root, &[mask])?.check()?;
        let grabs = Grabs::install(d)?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = thread::Builder::new()
            .name("comeback-input".into())
            .spawn(move || read_loop(grabs, target, base, ink, flag))
            .context("starting the input thread")?;
        Ok(Self { stop, handle })
    }

    pub fn finish(self) -> Captured {
        self.stop.store(true, Ordering::SeqCst);
        self.handle.join().unwrap_or_else(|_| Captured {
            events: Vec::new(),
            marks: Vec::new(),
            error: Some("the input thread panicked".into()),
        })
    }
}

fn read_loop(
    mut grabs: Grabs,
    target: [i32; 4],
    base: Instant,
    ink: Option<Sender<Ink>>,
    stop: Arc<AtomicBool>,
) -> Captured {
    let mut gestures = Gestures::new(target);
    let error = pump(&mut grabs, &mut gestures, base, &ink, &stop)
        .err()
        .map(|e| format!("{e:#}"));
    // Give the pointer back before anything slow happens.
    drop(grabs);
    let (events, marks) = gestures.finish();
    Captured {
        events,
        marks,
        error,
    }
}

/// Feeds `gestures` until `stop` is set or the X connection fails.
fn pump(
    grabs: &mut Grabs,
    gestures: &mut Gestures,
    base: Instant,
    ink: &Option<Sender<Ink>>,
    stop: &AtomicBool,
) -> Result<()> {
    // Buttons currently down, by number. Motion is only polled while one is.
    let mut down = 0u32;
    let bit = |button: u8| 1u32 << (button & 31);
    let mut last_poll = Instant::now();
    let now = || base.elapsed().as_millis() as u64;
    let pointer = |d: &Display| -> Result<(i32, i32, bool)> {
        let p = d.conn.query_pointer(d.root)?.reply()?;
        // Exactly Shift (ignoring the lock keys): any other modifier is not the draw gesture,
        // and the grab does not catch it either.
        let shift = p.mask.contains(KeyButMask::SHIFT) && (p.mask & other_modifiers()).bits() == 0;
        Ok((i32::from(p.root_x), i32::from(p.root_y), shift))
    };
    let send = |out: Option<Ink>| {
        if let (Some(tx), Some(i)) = (ink, out) {
            let _ = tx.send(i);
        }
    };
    while !stop.load(Ordering::SeqCst) {
        let Some(event) = grabs.d.conn.poll_for_event()? else {
            if down != 0 && last_poll.elapsed() >= MOTION_POLL {
                last_poll = Instant::now();
                let t = now();
                let (x, y, _) = pointer(&grabs.d)?;
                send(gestures.feed(Raw::Move { t, x, y }));
            } else {
                thread::sleep(Duration::from_millis(3));
            }
            continue;
        };
        match event {
            XEvent::XinputRawButtonPress(e) => {
                let t = now();
                let (x, y, shift) = pointer(&grabs.d)?;
                down |= bit(e.detail as u8);
                send(gestures.feed(Raw::Down {
                    t,
                    button: e.detail as u8,
                    x,
                    y,
                    shift,
                }));
            }
            // Only our own Shift+Button1 grab produces core button events, and the server
            // delivers the raw press first, so the gesture machine already knows whether this
            // press is ink (inside the target, exactly Shift) or the application's.
            XEvent::ButtonPress(_) => grabs.settle(!gestures.has_ink_press())?,
            XEvent::XinputRawButtonRelease(e) => {
                let t = now();
                let (x, y, _) = pointer(&grabs.d)?;
                let button = e.detail as u8;
                down &= !bit(button);
                send(gestures.feed(Raw::Up { t, button, x, y }));
            }
            // The end of a stroke we hold: while our own grab is active the server reports the
            // release to the grab owner as a core event and sends no raw one. The server ends
            // the grab by itself at this release.
            XEvent::ButtonRelease(e) => {
                down &= !bit(e.detail);
                send(gestures.feed(Raw::Up {
                    t: now(),
                    button: e.detail,
                    x: i32::from(e.root_x),
                    y: i32::from(e.root_y),
                }));
                if e.detail == 1 {
                    grabs.state = GrabState::Idle;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xinput_older_than_2_2_is_refused_with_the_reason() {
        for (major, minor) in [(2, 0), (2, 1), (1, 5)] {
            let err = require_xi(major, minor).unwrap_err().to_string();
            assert!(
                err.contains(&format!("offers XInput {major}.{minor}"))
                    && err.contains("needs 2.2")
                    && err.contains("pointer grab"),
                "{err}"
            );
        }
        for (major, minor) in [(2, 2), (2, 3), (3, 0)] {
            assert!(require_xi(major, minor).is_ok());
        }
    }
}
