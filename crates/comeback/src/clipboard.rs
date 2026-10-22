//! Putting the paste text on the clipboard.
//!
//! The spec names `arboard`, but on X11 its selection disappears when the process exits.
//! `xclip` and `xsel` fork a small server that outlives us, so they are tried first.

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Result, bail};

/// Copies `text` and returns the name of the tool that took it.
pub fn copy(text: &str) -> Result<&'static str> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let mut tools: Vec<(&'static str, &[&str])> = vec![
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    if wayland {
        tools.insert(0, ("wl-copy", &[]));
    }
    let mut last = String::from("none of xclip, xsel or wl-copy is installed");
    for (name, args) in tools {
        match feed(name, args, text) {
            Ok(()) => return Ok(name),
            Err(e) => last = format!("{name}: {e}"),
        }
    }
    bail!("{last}")
}

fn feed(name: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(name)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(text.as_bytes())?;
    let status = child.wait()?;
    if !status.success() {
        bail!("exited with {status}");
    }
    Ok(())
}
