//! Atomic writes and backups for Hive's own config files.
//!
//! Shared by the config and the folder-color store. Both use `std::fs` rather
//! than gio — the carve-out named in the build spec — so that they stay
//! GTK-free and testable without a display, and both touch only Hive's own
//! config directory. Write-then-rename is the only thing that keeps a settings
//! file from being truncated by a crash halfway through a save.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("could not create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Write `text` to `path` via a temporary file and a rename.
pub fn write(path: &Path, text: &str) -> Result<(), WriteError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| WriteError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let temp = temp_path(path);
    {
        let mut file = std::fs::File::create(&temp).map_err(|source| WriteError::Write {
            path: temp.clone(),
            source,
        })?;
        file.write_all(text.as_bytes())
            .map_err(|source| WriteError::Write {
                path: temp.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| WriteError::Write {
            path: temp.clone(),
            source,
        })?;
    }

    std::fs::rename(&temp, path).map_err(|source| {
        let _ = std::fs::remove_file(&temp);
        WriteError::Write {
            path: path.to_path_buf(),
            source,
        }
    })
}

/// `config.toml` -> `config.toml.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

/// Set an unusable file aside rather than overwriting what the user wrote.
pub fn back_up(path: &Path, contents: &str) -> Result<PathBuf, String> {
    let backup = backup_path(path);
    std::fs::write(&backup, contents)
        .map(|()| backup.clone())
        .map_err(|error| format!("could not write {}: {error}", backup.display()))
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_missing_parents_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deeper/thing.toml");
        write(&path, "a = 1\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 1\n");
        assert!(!temp_path(&path).exists());
    }

    #[test]
    fn write_replaces_an_existing_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thing.toml");
        write(&path, "first = true\n").unwrap();
        write(&path, "second = 1\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second = 1\n");
    }

    #[test]
    fn write_reports_an_error_rather_than_panicking() {
        assert!(write(Path::new("/proc/hive-nope/thing.toml"), "x = 1").is_err());
    }

    #[test]
    fn backups_sit_beside_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thing.toml");
        let backup = back_up(&path, "garbage").unwrap();
        assert_eq!(backup, backup_path(&path));
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "garbage");
    }
}
