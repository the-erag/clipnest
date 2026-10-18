//! The long-running Clipnest process: watches the clipboard and owns all windows.

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use crate::clipboard::Watcher;
use crate::config::Config;
use crate::menu::{build_menu, MenuActions, MenuPresenter};
use crate::paste::Paster;
use crate::popup::{Popup, PopupHost};
use crate::preferences::{Preferences, PrefsHost};
use crate::storage::Storage;
use crate::tray::Tray;
use crate::util::present;
use crate::{paths, system, APP_ID, APP_NAME};

/// Actions reachable over D-Bus, i.e. from `clipnest <command>` and hotkeys.
pub const COMMANDS: &[&str] = &["history", "search", "preferences", "clear", "quit"];

pub struct App {
    pub gtk: gtk::Application,
    pub config: Rc<Config>,
    pub storage: Rc<RefCell<Storage>>,
    paster: RefCell<Paster>,
    watcher: OnceCell<Rc<Watcher>>,
    popup: OnceCell<Rc<Popup>>,
    menu: OnceCell<Rc<MenuPresenter>>,
    tray: OnceCell<Rc<Tray>>,
    preferences: RefCell<Option<Rc<Preferences>>>,
    _hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl App {
    fn new(gtk_app: &gtk::Application) -> Rc<App> {
        paths::install_icons();
        if let Some(theme) = gtk::IconTheme::default() {
            theme.append_search_path(paths::icon_dir());
        }
        gtk::Window::set_default_icon_name("clipnest");

        let config = Rc::new(Config::load(None));
        system::first_run_setup(&config.get());
        let storage = match Storage::open(None) {
            Ok(storage) => storage,
            Err(err) => {
                eprintln!("clipnest: cannot open the history database: {err}");
                std::process::exit(1);
            }
        };
        storage.trim(config.get().max_history);
        let app = Rc::new(App {
            gtk: gtk_app.clone(),
            config,
            storage: Rc::new(RefCell::new(storage)),
            paster: RefCell::new(Paster::new()),
            watcher: OnceCell::new(),
            popup: OnceCell::new(),
            menu: OnceCell::new(),
            tray: OnceCell::new(),
            preferences: RefCell::new(None),
            _hold: RefCell::new(None),
        });
        app.setup();
        app
    }

    fn setup(self: &Rc<Self>) {
        let config = self.config.clone();
        let weak = Rc::downgrade(self);
        let watcher = Watcher::new(
            move || config.get(),
            self.storage.clone(),
            move || {
                if let Some(app) = weak.upgrade() {
                    app.history_changed(true);
                }
            },
        );
        let _ = self.watcher.set(watcher);

        let weak = Rc::downgrade(self);
        self.config.connect(move |key| {
            let Some(app) = weak.upgrade() else { return };
            if key == "max_history" {
                app.storage.borrow().trim(app.config.get().max_history);
                app.history_changed(true);
            } else if key.starts_with("menu_") {
                if let Some(tray) = app.tray.get() {
                    tray.schedule_rebuild();
                }
            }
        });

        for &name in COMMANDS {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    app.run_command(name);
                }
            });
            self.gtk.add_action(&action);
        }

        let _ = self.popup.set(Popup::new(&self.gtk, self.popup_host()));
        let (config, app) = (self.config.clone(), Rc::downgrade(self));
        let _ = self.menu.set(MenuPresenter::new(
            &self.gtk,
            move || config.get(),
            move |title_length| {
                app.upgrade()
                    .map(|app| app.menu_items(false, title_length))
                    .unwrap_or_default()
            },
        ));
        if let Some(connection) = self.gtk.dbus_connection() {
            let (build, activate, open) = (Rc::downgrade(self), Rc::downgrade(self), Rc::downgrade(self));
            let tray = Tray::new(
                connection,
                move || match build.upgrade() {
                    Some(app) => (app.menu_items(true, None), app.config.get().menu_show_images),
                    None => (Vec::new(), false),
                },
                Rc::new(move || {
                    if let Some(app) = activate.upgrade() {
                        app.toggle_popup();
                    }
                }),
                Rc::new(move |icon| {
                    if let Some(app) = open.upgrade() {
                        if let Some(popup) = app.popup.get() {
                            popup.hide();
                        }
                        if let Some(menu) = app.menu.get() {
                            menu.toggle_below(icon);
                        }
                    }
                }),
            );
            let _ = self.tray.set(tray);
        }

        for signal in [libc::SIGINT, libc::SIGTERM] {
            let weak = Rc::downgrade(self);
            glib::unix_signal_add_local(signal, move || {
                if let Some(app) = weak.upgrade() {
                    app.gtk.quit();
                }
                glib::ControlFlow::Break
            });
        }
        *self._hold.borrow_mut() = Some(self.gtk.hold()); // keep running with no window open
    }

    pub fn run_command(self: &Rc<Self>, command: &str) {
        match command {
            "history" => self.show_menu(),
            "search" => self.toggle_popup(),
            "preferences" => self.show_preferences(),
            "clear" => self.confirm_clear_history(None),
            "quit" => self.gtk.quit(),
            _ => {}
        }
    }

    fn popup_host(self: &Rc<Self>) -> PopupHost {
        let w = || Rc::downgrade(self);
        let (a, b, c, d, e, f, g, h) = (w(), w(), w(), w(), w(), w(), w(), w());
        PopupHost {
            history: Box::new(move || {
                a.upgrade()
                    .map(|app| app.storage.borrow().history(None))
                    .unwrap_or_default()
            }),
            image_path: Box::new(move |item| {
                b.upgrade()
                    .map(|app| app.storage.borrow().image_path(item))
                    .unwrap_or_default()
            }),
            can_auto_paste: Box::new(move || c.upgrade().map(|app| app.can_auto_paste()).unwrap_or(false)),
            use_item: Box::new(move |id, paste, plain| {
                if let Some(app) = d.upgrade() {
                    app.use_history_item(id, paste, false, plain);
                }
            }),
            set_pinned: Box::new(move |id, pinned| {
                if let Some(app) = e.upgrade() {
                    app.storage.borrow().set_pinned(id, pinned);
                }
            }),
            delete: Box::new(move |id| {
                if let Some(app) = f.upgrade() {
                    app.storage.borrow().delete(id);
                }
            }),
            changed: Box::new(move || {
                if let Some(app) = g.upgrade() {
                    app.history_changed(false);
                }
            }),
            preferences: Box::new(move || {
                if let Some(app) = h.upgrade() {
                    app.show_preferences();
                }
            }),
        }
    }

    /// How many history rows fit in the top-bar menu. GNOME draws that menu and
    /// cuts it off at the bottom of the screen instead of scrolling it, so it
    /// must fit; its submenus (the folders) do scroll.
    fn panel_rows() -> Option<usize> {
        let display = gdk::Display::default()?;
        let monitor = display.primary_monitor().or_else(|| display.monitor(0))?;
        let height = monitor.workarea().height() as f64;
        let interface = "org.gnome.desktop.interface";
        let text_scale = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup(interface, true))
            .map(|_| gio::Settings::new(interface).double("text-scaling-factor"))
            .filter(|scale| *scale > 0.0)
            .unwrap_or(1.0);
        // Rows are ~36 px at normal text size; leave room for the menu's
        // padding, separators, the "History" heading and the four actions.
        let row = 38.0 * text_scale;
        let rows = ((height - 60.0) / row).floor() as i64 - 1 - 4;
        Some(rows.max(3) as usize)
    }

    /// The history menu for the hotkey (`from_panel` false) or the top bar.
    fn menu_items(self: &Rc<Self>, from_panel: bool, title_length: Option<usize>) -> Vec<crate::menu::MenuItem> {
        let cfg = self.config.get();
        let w = || Rc::downgrade(self);
        let (a, b, c, d, e, f) = (w(), w(), w(), w(), w(), w());
        let actions = MenuActions {
            use_item: Rc::new(move |id, from_panel, plain| {
                if let Some(app) = a.upgrade() {
                    app.use_history_item(id, true, from_panel, plain);
                }
            }),
            search: Rc::new(move || {
                if let Some(app) = b.upgrade() {
                    app.toggle_popup();
                }
            }),
            clear: Rc::new(move || {
                if let Some(app) = c.upgrade() {
                    app.confirm_clear_history(None);
                }
            }),
            preferences: Rc::new(move || {
                if let Some(app) = d.upgrade() {
                    app.show_preferences();
                }
            }),
            quit: Rc::new(move || {
                if let Some(app) = e.upgrade() {
                    app.gtk.quit();
                }
            }),
            open_folder: Rc::new(move |items| {
                if let Some(menu) = f.upgrade().and_then(|app| app.menu.get().cloned()) {
                    menu.show_items(&items);
                }
            }),
        };
        let storage = self.storage.borrow();
        let entries = storage.history(Some(cfg.menu_max_items.max(0) as usize));
        let max_rows = if from_panel { Self::panel_rows() } else { None };
        build_menu(
            &cfg,
            &entries,
            &|item| storage.image_path(item),
            &actions,
            from_panel,
            title_length,
            max_rows,
        )
    }

    // -- picking items -------------------------------------------------------

    pub fn can_auto_paste(&self) -> bool {
        self.config.get().auto_paste && self.paster.borrow_mut().available()
    }

    fn paste_after_pick(self: &Rc<Self>, paste: bool, from_panel: bool) {
        if !paste || !self.can_auto_paste() {
            return;
        }
        let cfg = self.config.get();
        // The top-bar menu takes a little longer to hand focus back.
        let delay = cfg.paste_delay_ms + if from_panel { 150 } else { 0 };
        let wait = self.paster.borrow().wait_for(delay);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(wait, move || {
            if let Some(app) = weak.upgrade() {
                app.paster.borrow().paste(&app.config.get().paste_keys);
            }
        });
    }

    pub fn use_history_item(self: &Rc<Self>, id: i64, paste: bool, from_panel: bool, plain: bool) {
        let Some(item) = self.storage.borrow().get(id) else {
            return;
        };
        if let Some(watcher) = self.watcher.get() {
            if let Err(err) = watcher.set_item(&item, plain) {
                eprintln!("clipnest: could not restore item {id}: {err}");
                return;
            }
        }
        if self.config.get().reorder_on_select {
            self.storage.borrow().bump(id);
            self.history_changed(true);
        }
        self.paste_after_pick(paste, from_panel);
    }

    // -- change notifications -------------------------------------------------

    pub fn history_changed(&self, refresh_popup: bool) {
        if let Some(tray) = self.tray.get() {
            tray.schedule_rebuild();
        }
        if refresh_popup {
            if let Some(popup) = self.popup.get() {
                popup.history_changed();
            }
        }
    }

    fn set_hotkey(&self, which: &str, binding: &str) {
        let binding = binding.to_string();
        match which {
            "history" => self.config.update(|s| s.hotkey_history = binding.clone()),
            _ => self.config.update(|s| s.hotkey_search = binding.clone()),
        }
        if system::gnome_shortcuts_supported() {
            let shortcut = if which == "history" {
                &system::HISTORY
            } else {
                &system::SEARCH
            };
            system::set_gnome_shortcut(shortcut, &binding);
        }
    }

    // -- windows ---------------------------------------------------------------

    pub fn toggle_popup(&self) {
        if let Some(popup) = self.popup.get() {
            popup.toggle();
        }
    }

    pub fn show_menu(&self) {
        if let Some(popup) = self.popup.get() {
            popup.hide();
        }
        if let Some(menu) = self.menu.get() {
            menu.toggle();
        }
    }

    pub fn show_preferences(self: &Rc<Self>) {
        if let Some(popup) = self.popup.get() {
            popup.hide();
        }
        let existing = self.preferences.borrow().clone();
        let prefs = match existing {
            Some(prefs) => prefs,
            None => {
                let prefs = Preferences::new(&self.gtk, self.prefs_host());
                prefs.window.show_all();
                *self.preferences.borrow_mut() = Some(prefs.clone());
                prefs
            }
        };
        present(&prefs.window);
    }

    fn prefs_host(self: &Rc<Self>) -> PrefsHost {
        let w = || Rc::downgrade(self);
        let (a, b, c, d) = (w(), w(), w(), w());
        PrefsHost {
            config: self.config.clone(),
            paste_status: Box::new(move || {
                a.upgrade()
                    .map(|app| app.paster.borrow().describe())
                    .unwrap_or_default()
            }),
            clear_history: Box::new(move |window| {
                if let Some(app) = b.upgrade() {
                    app.confirm_clear_history(Some(window));
                }
            }),
            set_hotkey: Box::new(move |which, binding| {
                if let Some(app) = c.upgrade() {
                    app.set_hotkey(which, binding);
                }
            }),
            history_count: Box::new(move || d.upgrade().map(|app| app.storage.borrow().count()).unwrap_or(0)),
            data_dir: self.storage.borrow().directory.clone(),
        }
    }

    pub fn confirm_clear_history(&self, parent: Option<&gtk::Window>) {
        if let Some(popup) = self.popup.get() {
            popup.hide();
        }
        if self.config.get().confirm_clear {
            let dialog = gtk::MessageDialog::new(
                parent,
                gtk::DialogFlags::MODAL,
                gtk::MessageType::Question,
                gtk::ButtonsType::None,
                "Clear the clipboard history?",
            );
            dialog.set_secondary_text(Some("Pinned (★) items are kept."));
            dialog.add_buttons(&[
                ("_Cancel", gtk::ResponseType::Cancel),
                ("_Clear", gtk::ResponseType::Ok),
            ]);
            dialog.set_position(gtk::WindowPosition::Center);
            dialog.set_keep_above(true);
            present(&dialog);
            let response = dialog.run();
            dialog.close();
            if response != gtk::ResponseType::Ok {
                return;
            }
        }
        self.storage.borrow().clear(true);
        self.history_changed(true);
    }
}

thread_local! {
    static APP: OnceCell<Rc<App>> = const { OnceCell::new() };
}

pub fn run(command: &str) -> glib::ExitCode {
    let gtk_app = gtk::Application::new(Some(APP_ID), gio::ApplicationFlags::HANDLES_COMMAND_LINE);
    glib::set_application_name(APP_NAME);
    gtk_app.connect_startup(|gtk_app| {
        let app = App::new(gtk_app);
        APP.with(|cell| {
            let _ = cell.set(app);
        });
    });
    gtk_app.connect_command_line(|_, command_line| {
        let args = command_line.arguments();
        let command = args
            .get(1)
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_default();
        if COMMANDS.contains(&command.as_str()) {
            APP.with(|cell| {
                if let Some(app) = cell.get() {
                    app.run_command(&command);
                }
            });
        }
        0
    });
    let mut argv = vec![std::env::args().next().unwrap_or_else(|| "clipnest".into())];
    if !command.is_empty() {
        argv.push(command.to_string());
    }
    gtk_app.run_with_args(&argv)
}
