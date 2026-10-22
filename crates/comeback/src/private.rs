//! Files and directories that only their owner can open. A capture holds the screen and the
//! user's voice, so the process umask must not decide who else can read it.

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

/// Creates `path` and any missing parents with mode 0700. Directories that already exist keep
/// their mode: they belong to the user.
pub fn create_dir_all(path: &Path) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(path)
}

/// Creates (or empties) a file with mode 0600, so an encoder that opens it later writes into a
/// private file instead of creating one with the umask's mode.
pub fn create_file(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // `mode` is filtered by the umask on creation only; an existing file keeps its old mode.
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(file)
}
