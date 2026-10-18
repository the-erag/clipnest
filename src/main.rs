//! Clipnest: a clipboard history manager for Linux, by erag.

mod app;
mod clipboard;
mod config;
mod menu;
mod paste;
mod paths;
mod popup;
mod preferences;
mod status_icon;
mod storage;
mod system;
mod tray;
mod util;

use std::collections::HashMap;
use std::process::ExitCode;

use glib::{ToVariant, Variant};

pub const APP_ID: &str = "in.erag.Clipnest";
pub const APP_NAME: &str = "Clipnest";

const USAGE: &str = "Usage: clipnest [COMMAND]

Clipboard history for Linux, by erag (https://erag.in).

Commands:
  (none)            start Clipnest in the background (panel icon + history)
  history           show the clipboard history menu
  search            show/hide the searchable history popup
  preferences       open Preferences
  clear             clear the clipboard history (asks first)
  quit              stop the running Clipnest
  setup-shortcuts   register the global hotkeys with GNOME
  remove-shortcuts  remove Clipnest's GNOME hotkeys
  autostart on|off  launch Clipnest at login, or not
  version           print the version
";

/// Run an action in the already-running instance over D-Bus, without
/// starting GTK, so hotkeys are instant. False when Clipnest isn't running.
fn forward(command: &str) -> bool {
    let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) else {
        return false;
    };
    let args = (command, Vec::<Variant>::new(), HashMap::<String, Variant>::new()).to_variant();
    bus.call_sync(
        Some(APP_ID),
        &format!("/{}", APP_ID.replace('.', "/")),
        "org.gtk.Actions",
        "Activate",
        Some(&args),
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        3000,
        None::<&gio::Cancellable>,
    )
    .is_ok()
}

fn setup_shortcuts() -> ExitCode {
    let settings = config::Config::load(None).get();
    if !system::gnome_shortcuts_supported() {
        println!("GNOME keyboard settings not found. Bind these commands in your desktop's keyboard settings instead:");
        println!("  {} history\n  {} search", system::launcher(), system::launcher());
        return ExitCode::FAILURE;
    }
    system::apply_shortcuts(&settings);
    let show = |b: &str| {
        if b.is_empty() {
            "disabled".to_string()
        } else {
            b.to_string()
        }
    };
    println!("{}: {}", system::HISTORY.name, show(&settings.hotkey_history));
    println!("{}: {}", system::SEARCH.name, show(&settings.hotkey_search));
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    paths::migrate_from_clipora();
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let forwarded = ["history", "search", "preferences", "clear", "quit"];

    match command {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        "version" | "--version" => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        "setup-shortcuts" => return setup_shortcuts(),
        "remove-shortcuts" => {
            system::remove_shortcuts();
            return ExitCode::SUCCESS;
        }
        "autostart" => {
            return match args.get(2).map(String::as_str) {
                Some("on") => {
                    system::set_autostart(true);
                    ExitCode::SUCCESS
                }
                Some("off") => {
                    system::set_autostart(false);
                    ExitCode::SUCCESS
                }
                _ => {
                    eprint!("{USAGE}");
                    ExitCode::from(2)
                }
            };
        }
        "" | "daemon" => {}
        c if forwarded.contains(&c) => {
            if forward(c) {
                return ExitCode::SUCCESS;
            }
            if c == "quit" {
                println!("Clipnest is not running.");
                return ExitCode::SUCCESS;
            }
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    }

    // Wayland only lets the focused window read the clipboard, so run on
    // XWayland where clipboard changes can be watched from the background.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("CLIPNEST_NATIVE_WAYLAND").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    glib::set_prgname(Some("clipnest")); // WM_CLASS
    let command = if command == "daemon" { "" } else { command };
    match app::run(command).value() {
        0 => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}
