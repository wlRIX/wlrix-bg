// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the config file is.
//!
//! The convention is the whole stack's and this module only follows it: a hand-edited file under
//! `$XDG_CONFIG_HOME/wlrix/`, with `/etc/wlrix/` consulted when the user has none of their own,
//! and **the first file found wins outright** -- nothing is merged. See
//! `wlrix-compositor/src/config.rs`, `wlrix-desktop/src/xdg.rs` and
//! `wlrix-settings-daemon/src/paths.rs`; the lookup is duplicated in every component because the
//! repos build standalone, and this is one more copy rather than a new rule.
//!
//! The file's stem is `background`, not `bg`. It is the settings daemon's namespace as well as a
//! filename -- keys read `background.mode` -- and a stem that differs from the binary name has
//! precedent in `portal.toml`, which belongs to `xdg-desktop-portal-wlrix`.
//!
//! There is no state file. Nothing about a wallpaper is machine-written: unlike the desktop's
//! icon positions, there is nothing the user does at runtime for this program to remember.

use std::path::{Path, PathBuf};

/// The hand-edited settings file, relative to a config directory.
pub const CONFIG_NAME: &str = "wlrix/background.toml";
/// Consulted when the user has no config of their own.
const SYSTEM_CONFIG_DIR: &str = "/etc";

/// `$HOME`, or `None` when even that is unset.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// `$XDG_CONFIG_HOME`, or `~/.config` as the spec says to assume.
pub fn user_config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    home().map(|home| home.join(".config"))
}

/// Where to look for the settings file, most specific first.
pub fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = user_config_dir() {
        paths.push(dir.join(CONFIG_NAME));
    }
    paths.push(Path::new(SYSTEM_CONFIG_DIR).join(CONFIG_NAME));
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_path_is_the_one_the_daemon_writes_beside() {
        // Hand-kept in step with `wlrix-settings-daemon/src/paths.rs`, which resolves the same
        // two directories. A change here that is not mirrored there would have the daemon
        // writing somewhere this program never reads.
        let paths = config_paths();
        assert_eq!(
            paths.last().map(PathBuf::as_path),
            Some(Path::new("/etc/wlrix/background.toml"))
        );
        assert!(
            paths
                .first()
                .is_none_or(|path| path.ends_with("wlrix/background.toml"))
        );
    }

    #[test]
    fn the_user_path_comes_first() {
        // The precedence is the whole convention: the user's file shadows the system one
        // outright, so it has to be looked at first.
        if user_config_dir().is_some() {
            assert_eq!(config_paths().len(), 2);
        }
    }
}
