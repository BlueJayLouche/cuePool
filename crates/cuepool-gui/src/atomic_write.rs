//! Crash-safe file replacement.
//!
//! `std::fs::write` truncates first, so losing power or aborting partway
//! through leaves a truncated file. Writing a sibling temp file and renaming
//! it over the target means a reader (or the operator after a crash) sees
//! either the old file or the new one, never a half-written one.

use std::io;
use std::path::Path;

/// Write `data` to `path` via a temp file in the same directory and a rename.
///
/// Same directory as the target so the rename stays on one filesystem. The
/// pid keeps concurrent CuePool processes (automation profiles) off the same
/// temp name. On failure the temp file is removed and the target is left as
/// it was.
pub fn write_atomically(path: &Path, data: &[u8]) -> io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut temp_name = file_name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);
    std::fs::write(&temp, data)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cuepool-atomic-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writes_a_new_file_and_leaves_no_temp_behind() {
        let dir = scratch("new");
        let target = dir.join("show.qproj");
        write_atomically(&target, b"{}").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "only the target should remain");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_an_existing_file() {
        let dir = scratch("replace");
        let target = dir.join("show.qproj");
        std::fs::write(&target, b"old").unwrap();
        write_atomically(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rename_keeps_the_original_and_cleans_up() {
        let dir = scratch("fail");
        // A directory at the target path makes the rename fail on every OS.
        let target = dir.join("show.qproj");
        std::fs::create_dir(&target).unwrap();
        assert!(write_atomically(&target, b"x").is_err());
        assert!(target.is_dir(), "target untouched");
        let leftovers: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(leftovers.len(), 1, "temp file removed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
