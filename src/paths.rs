//! Where Clipnest keeps its files.
//!
//! Set CLIPNEST_HOME to keep config and data in one throwaway directory
//! (handy for development and tests).

use std::env;
use std::path::PathBuf;

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => glib::home_dir().join(fallback),
    }
}

pub fn config_dir() -> PathBuf {
    match env::var_os("CLIPNEST_HOME") {
        Some(home) => PathBuf::from(home).join("config"),
        None => xdg("XDG_CONFIG_HOME", ".config").join("clipnest"),
    }
}

pub fn data_dir() -> PathBuf {
    match env::var_os("CLIPNEST_HOME") {
        Some(home) => PathBuf::from(home).join("data"),
        None => xdg("XDG_DATA_HOME", ".local/share").join("clipnest"),
    }
}

/// Clipnest used to be called Clipora: move its settings and history over the
/// first time Clipnest runs.
pub fn migrate_from_clipora() {
    if env::var_os("CLIPNEST_HOME").is_some() {
        return;
    }
    let moves = [
        (xdg("XDG_CONFIG_HOME", ".config").join("clipora"), config_dir()),
        (xdg("XDG_DATA_HOME", ".local/share").join("clipora"), data_dir()),
    ];
    for (old, new) in moves {
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
    let data = data_dir();
    for suffix in ["", "-wal", "-shm"] {
        let old = data.join(format!("clipora.db{suffix}"));
        let new = data.join(format!("clipnest.db{suffix}"));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(old, new);
        }
    }
    for icon in ["clipora.svg", "clipora-symbolic.svg", "clipora-panel.svg"] {
        let _ = std::fs::remove_file(data.join("icons").join(icon));
    }
}

pub fn autostart_dir() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart")
}

/// Icons are compiled in and written next to the data, where both GTK and the
/// top-bar icon host can find them.
pub fn icon_dir() -> PathBuf {
    data_dir().join("icons")
}

pub const ICON_SYMBOLIC: &[u8] = include_bytes!("../data/icons/clipnest-symbolic.svg");
pub const ICON_APP: &[u8] = include_bytes!("../data/icons/clipnest.svg");
/// White, for the dark GNOME top bar: a legacy tray icon isn't recoloured.
pub const ICON_PANEL: &[u8] = include_bytes!("../data/icons/clipnest-panel.svg");

pub fn install_icons() {
    let dir = icon_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    for (name, data) in [
        ("clipnest-symbolic.svg", ICON_SYMBOLIC),
        ("clipnest.svg", ICON_APP),
        ("clipnest-panel.svg", ICON_PANEL),
    ] {
        let path = dir.join(name);
        if std::fs::read(&path).ok().as_deref() != Some(data) {
            let _ = std::fs::write(&path, data);
        }
    }
}
