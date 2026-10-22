//! `comeback record`: headless capture without the daemon.

use std::io::{BufRead, IsTerminal};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use comeback_core::model::TargetKind;

use crate::session::{Options, Session};
use crate::target::{clamp_to_screen, fit_area, parse_area};
use crate::x11;

pub struct Args {
    pub window: bool,
    pub area: Option<String>,
    pub duration: Option<f64>,
    pub out: Option<PathBuf>,
    pub no_clipboard: bool,
}

static STOP: AtomicBool = AtomicBool::new(false);

pub fn run(args: Args, captures: PathBuf) -> Result<()> {
    // Validate everything that needs no X server first, so a typo fails instantly.
    let area = args.area.as_deref().map(parse_area).transpose()?;
    if let Some(d) = args.duration
        && !(d.is_finite() && d > 0.0)
    {
        bail!("--duration must be a positive number of seconds, got {d}");
    }
    let display = x11::connect()?;
    let screen = x11::screen_rect(&display)?;
    let (kind, rect, app, title) = if let Some(a) = area {
        (TargetKind::Area, fit_area(a, screen)?, None, None)
    } else if args.window {
        eprintln!("comeback: click the window to record");
        let p = x11::pick_window(&display)?;
        (
            TargetKind::Window,
            fit_area(clamp_to_screen(p.rect, screen)?, screen)?,
            p.app,
            p.title,
        )
    } else {
        let monitor = x11::monitor_under_pointer(&display)?;
        (TargetKind::Screen, fit_area(monitor, screen)?, None, None)
    };
    drop(display);

    ctrlc::set_handler(|| STOP.store(true, Ordering::SeqCst))
        .context("installing the Ctrl-C handler")?;
    if std::io::stdin().is_terminal() {
        thread::spawn(|| {
            let mut line = String::new();
            if std::io::stdin().lock().read_line(&mut line).is_ok() {
                STOP.store(true, Ordering::SeqCst);
            }
        });
    }

    let mut session = Session::start(Options {
        captures,
        out: args.out,
        kind,
        rect,
        app,
        title,
        clipboard: !args.no_clipboard,
    })?;
    for note in session.notes() {
        eprintln!("comeback: note: {note}");
    }
    eprintln!(
        "comeback: recording {}x{} at {},{}; Shift-drag draws ink; {}",
        rect[2],
        rect[3],
        rect[0],
        rect[1],
        if args.duration.is_some() {
            "stops by itself (Ctrl-C stops early)"
        } else {
            "press Enter or Ctrl-C to stop"
        }
    );
    let deadline = args
        .duration
        .map(|d| Instant::now() + Duration::from_secs_f64(d));
    while !STOP.load(Ordering::SeqCst)
        && deadline.is_none_or(|t| Instant::now() < t)
        && session.is_alive()
    {
        thread::sleep(Duration::from_millis(50));
    }
    eprintln!("comeback: stopping, compiling the bundle");
    let done = session.finish()?;
    match &done.clipboard {
        Some(Ok(tool)) => eprintln!("comeback: copied to the clipboard ({tool})"),
        Some(Err(e)) => eprintln!("comeback: clipboard copy failed ({e})"),
        None => {}
    }
    print!("{}", done.paste);
    Ok(())
}
