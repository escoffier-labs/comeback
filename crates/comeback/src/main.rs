mod demo;

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
    /// Print a bundle's paste text, or its manifest with --json.
    Inspect {
        #[arg(default_value = "latest")]
        bundle: String,
        #[arg(long)]
        json: bool,
    },
}

fn captures_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("COMEBACK_CAPTURES_DIR") {
        return PathBuf::from(d);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        });
    data.join("comeback/captures")
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
            let m = demo::run(&out, id)?;
            print!("{}", summary::paste_text(&m, &out));
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
