//! Desktop integration: global shortcuts and launch-at-login.
//!
//! Wayland clients cannot grab global hotkeys, so Clipnest registers GNOME
//! custom keyboard shortcuts that run `clipnest history` / `clipnest search`.

use std::path::PathBuf;

use gio::prelude::*;

use crate::config::Settings;
use crate::paths;
use crate::{APP_ID, APP_NAME};

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_KEY: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

pub struct Shortcut {
    pub slot: &'static str,
    pub name: &'static str,
    pub command: &'static str,
}

pub const HISTORY: Shortcut = Shortcut {
    slot: "clipnest-history",
    name: "Clipnest: clipboard menu",
    command: "history",
};
pub const SEARCH: Shortcut = Shortcut {
    slot: "clipnest-search",
    name: "Clipnest: search history",
    command: "search",
};
/// Shortcuts earlier versions registered (from when the app was called
/// Clipora); removed when shortcuts are applied.
const RETIRED_SLOTS: &[&str] = &["clipora-history", "clipora-search", "clipora-snippets"];

fn custom_path(slot: &str) -> String {
    format!("/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/{slot}/")
}

/// The command that starts this copy of Clipnest.
pub fn launcher() -> String {
    if let Some(wrapper) = std::env::var_os("CLIPNEST_LAUNCHER") {
        return wrapper.to_string_lossy().into_owned();
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clipnest".into())
}

pub fn gnome_shortcuts_supported() -> bool {
    gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup(MEDIA_KEYS, true))
        .is_some()
}

fn registered(media: &gio::Settings) -> Vec<String> {
    media.strv("custom-keybindings").iter().map(|s| s.to_string()).collect()
}

fn set_registered(media: &gio::Settings, paths: &[String]) {
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    let _ = media.set_strv("custom-keybindings", refs.as_slice());
}

fn remove_slot(slot: &str) {
    let media = gio::Settings::new(MEDIA_KEYS);
    let path = custom_path(slot);
    let mut paths = registered(&media);
    if paths.contains(&path) {
        paths.retain(|p| p != &path);
        set_registered(&media, &paths);
    }
    let s = gio::Settings::with_path(CUSTOM_KEY, &path);
    for key in ["name", "command", "binding"] {
        s.reset(key);
    }
    gio::Settings::sync();
}

/// Create/update (or remove, if binding is empty) one GNOME custom shortcut.
pub fn set_gnome_shortcut(shortcut: &Shortcut, binding: &str) {
    if binding.is_empty() {
        remove_slot(shortcut.slot);
        return;
    }
    let media = gio::Settings::new(MEDIA_KEYS);
    let path = custom_path(shortcut.slot);
    let s = gio::Settings::with_path(CUSTOM_KEY, &path);
    let _ = s.set_string("name", shortcut.name);
    let _ = s.set_string("command", &format!("{} {}", launcher(), shortcut.command));
    let _ = s.set_string("binding", binding);
    let mut paths = registered(&media);
    if !paths.contains(&path) {
        paths.push(path);
        set_registered(&media, &paths);
    }
    gio::Settings::sync();
}

pub fn apply_shortcuts(settings: &Settings) {
    for slot in RETIRED_SLOTS {
        remove_slot(slot);
    }
    set_gnome_shortcut(&HISTORY, &settings.hotkey_history);
    set_gnome_shortcut(&SEARCH, &settings.hotkey_search);
}

pub fn remove_shortcuts() {
    for slot in RETIRED_SLOTS.iter().chain([HISTORY.slot, SEARCH.slot].iter()) {
        remove_slot(slot);
    }
}

// -- launch at login ----------------------------------------------------------

fn autostart_file() -> PathBuf {
    paths::autostart_dir().join(format!("{APP_ID}.desktop"))
}

/// The entry the .deb package installs for every user, if any.
fn system_autostart_file() -> Option<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS").unwrap_or_else(|_| "/etc/xdg".into());
    dirs.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("autostart").join(format!("{APP_ID}.desktop")))
        .find(|path| path.exists())
}

pub fn autostart_enabled() -> bool {
    match std::fs::read_to_string(autostart_file()) {
        Ok(text) => !text.contains("Hidden=true") && !text.contains("X-GNOME-Autostart-enabled=false"),
        Err(_) => system_autostart_file().is_some(),
    }
}

pub fn set_autostart(enabled: bool) {
    let path = autostart_file();
    let system_wide = system_autostart_file().is_some();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if !enabled {
        if system_wide {
            // A user entry with Hidden=true switches the system-wide one off.
            let _ = std::fs::write(
                &path,
                format!("[Desktop Entry]\nType=Application\nName={APP_NAME}\nHidden=true\n"),
            );
        } else {
            let _ = std::fs::remove_file(path);
        }
        return;
    }
    if system_wide {
        let _ = std::fs::remove_file(path); // the system-wide entry starts it
        return;
    }
    let _ = std::fs::write(
        path,
        format!(
            "[Desktop Entry]\nType=Application\nName={APP_NAME}\nComment=Clipboard history manager\n\
             Exec={}\nIcon=clipnest\nTerminal=false\nNoDisplay=true\n\
             X-GNOME-Autostart-enabled=true\nX-GNOME-Autostart-Delay=2\n",
            launcher()
        ),
    );
}

// -- first run ------------------------------------------------------------------

const EXTENSION_UUID: &str = "clipnest@erag.in";

/// Switch on the "Clipnest helper" GNOME Shell extension for this user.
pub fn enable_extension() {
    let schema = "org.gnome.shell";
    if gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup(schema, true))
        .is_none()
    {
        return;
    }
    let shell = gio::Settings::new(schema);
    let mut enabled: Vec<String> = shell.strv("enabled-extensions").iter().map(|s| s.to_string()).collect();
    if !enabled.iter().any(|uuid| uuid == EXTENSION_UUID) {
        enabled.push(EXTENSION_UUID.into());
        let refs: Vec<&str> = enabled.iter().map(String::as_str).collect();
        let _ = shell.set_strv("enabled-extensions", refs.as_slice());
        gio::Settings::sync();
    }
}

/// The first time Clipnest runs for a user, set up what an installer can't do
/// for every account at once (the .deb package runs as root): the hotkeys, the
/// helper extension and launch at login.
pub fn first_run_setup(settings: &Settings) {
    if std::env::var_os("CLIPNEST_HOME").is_some() {
        return; // development/test instance: leave the real desktop alone
    }
    let marker = paths::config_dir().join("initialized");
    if marker.exists() {
        return;
    }
    if gnome_shortcuts_supported() {
        apply_shortcuts(settings);
    }
    enable_extension();
    if !autostart_enabled() {
        set_autostart(true);
    }
    let _ = std::fs::create_dir_all(paths::config_dir());
    let _ = std::fs::write(marker, env!("CARGO_PKG_VERSION"));
}
