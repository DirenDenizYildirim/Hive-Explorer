//! XDG location resolution.

use std::path::PathBuf;

/// Application identifier, and therefore the Wayland `app_id` window rules target.
pub const APP_ID: &str = "dev.diren.Hive";

/// Subdirectory name used under each XDG root.
const DIR_NAME: &str = "hive";

/// `$XDG_CONFIG_HOME/hive/`
pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join(DIR_NAME)
}

/// `$XDG_CONFIG_HOME/hive/config.toml`
pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

/// `$XDG_CONFIG_HOME/hive/themes/` — user themes. Ships empty.
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

/// `$XDG_CONFIG_HOME/hive/folder-colors.toml`
pub fn folder_colors_file() -> PathBuf {
    config_dir().join("folder-colors.toml")
}

/// `$XDG_CACHE_HOME/hive/thumbnails/`
pub fn thumbnail_cache_dir() -> PathBuf {
    glib::user_cache_dir().join(DIR_NAME).join("thumbnails")
}

/// `$XDG_STATE_HOME/hive/logs/`
pub fn log_dir() -> PathBuf {
    state_dir().join(DIR_NAME).join("logs")
}

fn state_dir() -> PathBuf {
    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        let path = PathBuf::from(state);
        if path.is_absolute() {
            return path;
        }
    }
    glib::home_dir().join(".local").join("state")
}

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    glib::home_dir()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn app_id_is_the_documented_string() {
        assert_eq!(APP_ID, "dev.diren.Hive");
    }

    #[test]
    fn paths_are_absolute_and_namespaced() {
        for path in [
            config_dir(),
            config_file(),
            themes_dir(),
            folder_colors_file(),
            thumbnail_cache_dir(),
            log_dir(),
        ] {
            assert!(path.is_absolute(), "{path:?} should be absolute");
            assert!(
                path.components().any(|c| c.as_os_str() == "hive"),
                "{path:?} should be namespaced under hive/"
            );
        }
    }

    #[test]
    fn config_file_sits_in_the_config_dir() {
        assert_eq!(config_file().parent(), Some(config_dir().as_path()));
        assert_eq!(themes_dir().parent(), Some(config_dir().as_path()));
    }
}
