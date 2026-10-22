//! The ink overlay: an override-redirect window shaped to exactly the stroke pixels.
//!
//! Its input region is empty, so it never takes a click.
//!
//! Shaping works without a compositing manager (an ARGB window would be opaque under Xvfb and
//! many window managers) and the screen grab sees the ink, so video and keyframes carry it.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    CapStyle, ClipOrdering, ConfigureWindowAux, ConnectionExt as _, CoordMode, CreateGCAux,
    CreateWindowAux, JoinStyle, Point, Rectangle, StackMode, WindowClass,
};

use crate::input::Ink;
use crate::x11;

/// How long strokes stay after the last one ends.
const LINGER: Duration = Duration::from_secs(3);
const PEN_WIDTH: u32 = 6;
/// Ink colour, 16 bits per channel (230, 30, 30).
const RED: (u16, u16, u16) = (0xE6E6, 0x1E1E, 0x1E1E);

pub struct Overlay {
    tx: Sender<Ink>,
    handle: JoinHandle<Result<()>>,
}

impl Overlay {
    /// Creates the (initially invisible) window on the calling thread, then serves it from a thread.
    pub fn spawn(target: [i32; 4]) -> Result<Self> {
        let d = x11::connect()?;
        let [tx_, ty, tw, th] = target;
        let (w, h) = (tw as u16, th as u16);
        d.conn
            .shape_query_version()?
            .reply()
            .context("the SHAPE extension is not available")?;
        let screen = &d.conn.setup().roots[0];
        let pixel = d
            .conn
            .alloc_color(screen.default_colormap, RED.0, RED.1, RED.2)?
            .reply()?
            .pixel;
        let win = d.conn.generate_id()?;
        d.conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            win,
            d.root,
            tx_ as i16,
            ty as i16,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(pixel),
        )?;
        d.conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::BOUNDING,
            ClipOrdering::UNSORTED,
            win,
            0,
            0,
            &[],
        )?;
        // Ink is a picture, not a surface: with an empty input region every click, scroll and
        // drag goes through to the window underneath, ink or not.
        d.conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            win,
            0,
            0,
            &[],
        )?;
        d.conn.map_window(win)?;
        d.conn.flush()?;
        let (tx, rx) = mpsc::channel::<Ink>();
        let handle = thread::Builder::new()
            .name("comeback-ink".into())
            .spawn(move || -> Result<()> {
                let mask = d.conn.generate_id()?;
                d.conn.create_pixmap(1, mask, win, w, h)?;
                let gc = d.conn.generate_id()?;
                d.conn
                    .create_gc(gc, mask, &CreateGCAux::new().foreground(0))?;
                let mut strokes: Vec<Vec<Point>> = Vec::new();
                let mut drawing = false;
                let mut last_activity = Instant::now();
                let mut dirty = false;
                let local = |p: [i32; 2]| Point {
                    x: (p[0] - tx_) as i16,
                    y: (p[1] - ty) as i16,
                };
                let result = loop {
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(Ink::Begin(p)) => {
                            strokes.push(vec![local(p)]);
                            drawing = true;
                            dirty = true;
                        }
                        Ok(Ink::Point(p)) => {
                            if let Some(s) = strokes.last_mut() {
                                s.push(local(p));
                                dirty = true;
                            }
                        }
                        Ok(Ink::End) => {
                            drawing = false;
                            last_activity = Instant::now();
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break Ok(()),
                    }
                    if !drawing && !strokes.is_empty() && last_activity.elapsed() > LINGER {
                        strokes.clear();
                        dirty = true;
                    }
                    if dirty {
                        dirty = false;
                        if let Err(e) = redraw(&d, win, mask, gc, (w, h), &strokes) {
                            break Err(e);
                        }
                    }
                };
                let _ = d.conn.destroy_window(win);
                let _ = d.conn.free_pixmap(mask);
                let _ = d.conn.flush();
                result
            })
            .context("starting the ink thread")?;
        Ok(Self { tx, handle })
    }

    pub fn sender(&self) -> Sender<Ink> {
        self.tx.clone()
    }

    /// Removes the overlay window and reports any error the thread hit.
    pub fn close(self) -> Result<()> {
        drop(self.tx);
        self.handle
            .join()
            .map_err(|_| anyhow!("ink thread panicked"))?
    }
}

fn redraw(
    d: &x11::Display,
    win: u32,
    mask: u32,
    gc: u32,
    (w, h): (u16, u16),
    strokes: &[Vec<Point>],
) -> Result<()> {
    let c = &d.conn;
    c.change_gc(
        gc,
        &x11rb::protocol::xproto::ChangeGCAux::new().foreground(0),
    )?;
    c.poly_fill_rectangle(
        mask,
        gc,
        &[Rectangle {
            x: 0,
            y: 0,
            width: w,
            height: h,
        }],
    )?;
    c.change_gc(
        gc,
        &x11rb::protocol::xproto::ChangeGCAux::new()
            .foreground(1)
            .line_width(PEN_WIDTH)
            .cap_style(CapStyle::ROUND)
            .join_style(JoinStyle::ROUND),
    )?;
    for s in strokes {
        if s.len() >= 2 {
            c.poly_line(CoordMode::ORIGIN, mask, gc, s)?;
        }
    }
    c.shape_mask(shape::SO::SET, shape::SK::BOUNDING, win, 0, 0, mask)?;
    c.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
    c.flush()?;
    Ok(())
}
