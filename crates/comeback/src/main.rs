mod clipboard;
mod daemon;
mod demo;
mod gst;
mod ink;
mod input;
mod media;
mod private;
mod record;
mod session;
mod target;
mod whisper;
mod x11;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::{Parser, Subcommand};
use comeback_core::{bundle, summary};

#[derive(Parser)]
#[command(
    name = "comeback",
    version,
    about = "Show an agent the problem: screen, voice and ink in one bundle"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build a synthetic bundle without capture hardware and print its paste text.
    Demo {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Record the screen, microphone, clicks and Shift-drag ink into a bundle (X11).
    Record {
        /// Record one window: click it when asked.
        #[arg(long, conflicts_with = "area")]
        window: bool,
        /// Record a rectangle of the screen.
        #[arg(long, value_name = "X,Y,W,H")]
        area: Option<String>,
        /// Stop by itself after this many seconds.
        #[arg(long, value_name = "SECONDS")]
        duration: Option<f64>,
        /// Write the bundle here instead of the captures directory. Must not exist.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Do not copy the paste text to the clipboard.
        #[arg(long)]
        no_clipboard: bool,
    },
    /// Run in the background: Alt+Shift+S starts and stops a recording of the monitor under the pointer.
    Daemon,
    /// Start or stop a recording through the running daemon, like pressing Alt+Shift+S.
    Toggle,
    /// Discard the recording in progress, leaving every finished capture alone.
    Cancel,
    /// Say whether the daemon is idle or recording.
    Status,
    /// Print the agent skill that explains bundles, or install it as <DIR>/comeback/SKILL.md.
    Skill {
        /// A skills directory such as ~/.claude/skills.
        #[arg(long, value_name = "DIR")]
        install: Option<PathBuf>,
    },
    /// Print a bundle's paste text, or its manifest with --json.
    Inspect {
        #[arg(default_value = "latest")]
        bundle: String,
        #[arg(long)]
        json: bool,
    },
}

/// The agent skill, as shipped in `skills/comeback/SKILL.md`.
const SKILL: &str = include_str!("../../../skills/comeback/SKILL.md");

/// `$XDG_DATA_HOME`, or `~/.local/share`.
fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
}

fn captures_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("COMEBACK_CAPTURES_DIR") {
        return PathBuf::from(d);
    }
    data_dir().join("comeback/captures")
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::Demo { out } => {
            let id = format!("comeback-{}", now_ms());
            let out = out.unwrap_or_else(|| captures_dir().join(format!("{id}.comeback")));
            if out.exists() {
                anyhow::bail!("{} already exists", out.display());
            }
            let m = demo::run(&out, id)?;
            print!("{}", summary::paste_text(&m, &out));
        }
        Cmd::Record {
            window,
            area,
            duration,
            out,
            no_clipboard,
        } => {
            record::run(
                record::Args {
                    window,
                    area,
                    duration,
                    out,
                    no_clipboard,
                },
                captures_dir(),
            )?;
        }
        Cmd::Daemon => daemon::run(captures_dir())?,
        Cmd::Toggle => daemon::client("toggle")?,
        Cmd::Cancel => daemon::client("cancel")?,
        Cmd::Status => daemon::client("status")?,
        Cmd::Skill { install: None } => print!("{SKILL}"),
        Cmd::Skill { install: Some(dir) } => {
            let file = dir.join("comeback").join("SKILL.md");
            if let Ok(old) = std::fs::read_to_string(&file)
                && old != SKILL
            {
                anyhow::bail!(
                    "{} already exists and differs; remove it first if you want the bundled version",
                    file.display()
                );
            }
            std::fs::create_dir_all(file.parent().expect("has a parent"))?;
            std::fs::write(&file, SKILL)?;
            println!("installed {}", file.display());
        }
        Cmd::Inspect { bundle: arg, json } => {
            let dir = bundle::resolve(&arg, &captures_dir())?;
            let m = bundle::load(&dir)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&m)?);
            } else {
                print!("{}", summary::paste_text(&m, &dir));
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("comeback: {e:#}");
            ExitCode::from(1)
        }
    }
}
