use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::model::{Manifest, SCHEMA};

pub fn load(dir: &Path) -> Result<Manifest> {
    let path = dir.join("comeback.json");
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let raw: serde_json::Value =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    let schema = raw.get("schema").and_then(|v| v.as_u64()).unwrap_or(0);
    if schema != SCHEMA as u64 {
        bail!("unsupported bundle schema {schema} (this comeback reads schema {SCHEMA})");
    }
    serde_json::from_value(raw).with_context(|| format!("parsing {}", path.display()))
}

/// The finished capture directory (one with a `comeback.json`) with the largest
/// `comeback-<ms>.comeback` timestamp.
pub fn latest(captures: &Path) -> Result<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    if let Ok(entries) = fs::read_dir(captures) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(ms) = name
                .strip_prefix("comeback-")
                .and_then(|r| r.strip_suffix(".comeback"))
                .and_then(|r| r.parse::<u64>().ok())
            else {
                continue;
            };
            if !e.path().join("comeback.json").is_file() {
                continue;
            }
            if best.as_ref().is_none_or(|(b, _)| ms > *b) {
                best = Some((ms, e.path()));
            }
        }
    }
    match best {
        Some((_, p)) => Ok(p),
        None => bail!("no captures in {}", captures.display()),
    }
}

/// `latest` or a path.
pub fn resolve(arg: &str, captures: &Path) -> Result<PathBuf> {
    if arg == "latest" {
        latest(captures)
    } else {
        Ok(PathBuf::from(arg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_picks_the_newest_capture() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "comeback-100.comeback",
            "comeback-900.comeback",
            "comeback-50.comeback",
            "notes.txt",
        ] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
            if name.starts_with("comeback-") {
                std::fs::write(dir.path().join(name).join("comeback.json"), "").unwrap();
            }
        }
        // newer by name but never finished: no comeback.json
        std::fs::create_dir(dir.path().join("comeback-999.comeback")).unwrap();
        assert_eq!(
            latest(dir.path()).unwrap(),
            dir.path().join("comeback-900.comeback")
        );
        assert_eq!(
            resolve("latest", dir.path()).unwrap(),
            dir.path().join("comeback-900.comeback")
        );
        assert_eq!(
            resolve("/x/y.comeback", dir.path()).unwrap(),
            PathBuf::from("/x/y.comeback")
        );
    }

    #[test]
    fn latest_errors_on_an_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let err = latest(dir.path()).unwrap_err().to_string();
        assert!(err.starts_with("no captures in"), "{err}");
    }

    #[test]
    fn load_rejects_other_schemas() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("comeback.json"), r#"{"schema": 2}"#).unwrap();
        let err = load(dir.path()).unwrap_err().to_string();
        assert_eq!(
            err,
            "unsupported bundle schema 2 (this comeback reads schema 1)"
        );
    }
}
