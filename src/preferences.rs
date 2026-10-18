//! Preferences window with Clipy's tabs: General, Menu, Type, Exclude, Shortcuts, About.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use regex::Regex;

use crate::config::{Config, Settings, PASTE_KEY_CHOICES};
use crate::system;
use crate::util::thousands;

const TOOLBAR_CSS: &str = "
.clipnest-prefs-toolbar {
    padding: 6px 12px 10px 12px;
    border-bottom: 1px solid alpha(@theme_fg_color, 0.12);
}
.clipnest-prefs-tab, .clipnest-prefs-tab:backdrop {
    background: none; border: none; box-shadow: none;
    border-radius: 8px; padding: 6px 8px; min-width: 66px;
}
.clipnest-prefs-tab:hover { background-color: alpha(@theme_fg_color, 0.06); }
.clipnest-prefs-tab:checked { background-color: alpha(@theme_fg_color, 0.10); }
.clipnest-prefs-tab:checked image, .clipnest-prefs-tab:checked label {
    color: @theme_selected_bg_color;
}
";

/// (page id, title, icon) in Clipy's order. Clipy's last tab is "Updates";
/// Clipnest is installed from source, so that slot is "About".
const PAGES: &[(&str, &str, &str)] = &[
    ("general", "General", "emblem-system-symbolic"),
    ("menu", "Menu", "view-list-symbolic"),
    ("type", "Type", "edit-copy-symbolic"),
    ("exclude", "Exclude", "action-unavailable-symbolic"),
    ("shortcuts", "Shortcuts", "input-keyboard-symbolic"),
    ("about", "About", "help-about-symbolic"),
];

/// Records a hotkey: (which: "history" | "search", binding).
pub type HotkeySetter = Box<dyn Fn(&str, &str)>;

/// What the window needs from the rest of the app.
pub struct PrefsHost {
    pub config: Rc<Config>,
    pub paste_status: Box<dyn Fn() -> String>,
    pub clear_history: Box<dyn Fn(&gtk::Window)>,
    pub set_hotkey: HotkeySetter,
    pub history_count: Box<dyn Fn() -> i64>,
    pub data_dir: std::path::PathBuf,
}

pub struct Preferences {
    pub window: gtk::Window,
    host: Rc<PrefsHost>,
    header: gtk::HeaderBar,
    stack: gtk::Stack,
    spins: RefCell<Vec<gtk::SpinButton>>,
    stats: gtk::Label,
}

fn grid() -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(24);
    grid.set_row_spacing(6);
    grid.set_border_width(24);
    grid
}

fn dim(label: &gtk::Label) {
    label.style_context().add_class("dim-label");
}

fn wrapped(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_max_width_chars(60);
    label
}

/// Grid rows are appended one after another.
struct Rows {
    grid: gtk::Grid,
    next: Cell<i32>,
}

impl Rows {
    fn new() -> Rows {
        Rows {
            grid: grid(),
            next: Cell::new(0),
        }
    }

    fn take(&self) -> i32 {
        let row = self.next.get();
        self.next.set(row + 1);
        row
    }

    fn row(&self, label: &str, widget: &impl IsA<gtk::Widget>, hint: Option<&str>) {
        let title = wrapped(label);
        title.set_hexpand(true);
        widget.set_halign(gtk::Align::End);
        widget.set_valign(gtk::Align::Center);
        let row = self.take();
        self.grid.attach(&title, 0, row, 1, 1);
        self.grid.attach(widget, 1, row, 1, 1);
        if let Some(hint) = hint {
            let hint = wrapped(hint);
            dim(&hint);
            self.grid.attach(&hint, 0, self.take(), 2, 1);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_margin_top(6);
        self.grid.attach(&spacer, 0, self.take(), 2, 1);
    }

    fn note(&self, text: &str) -> gtk::Label {
        let note = wrapped(text);
        dim(&note);
        note.set_selectable(true);
        self.grid.attach(&note, 0, self.take(), 2, 1);
        note
    }
}

impl Preferences {
    pub fn new(app: &gtk::Application, host: PrefsHost) -> Rc<Preferences> {
        let window = gtk::Window::builder().application(app).title("General").build();
        window.set_default_size(660, 600);
        window.set_position(gtk::WindowPosition::Center);
        window.set_icon_name(Some("clipnest"));
        window.connect_delete_event(|w, _| {
            w.hide();
            glib::Propagation::Stop
        });
        if let Some(screen) = GtkWindowExt::screen(&window) {
            let provider = gtk::CssProvider::new();
            let _ = provider.load_from_data(TOOLBAR_CSS.as_bytes());
            gtk::StyleContext::add_provider_for_screen(&screen, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        // Title bar shows the current tab's name, like Clipy's.
        let header = gtk::HeaderBar::new();
        header.set_show_close_button(true);
        header.set_title(Some("General"));
        window.set_titlebar(Some(&header));

        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(120);
        let stats = gtk::Label::new(None);
        dim(&stats);

        let prefs = Rc::new(Preferences {
            window,
            host: Rc::new(host),
            header,
            stack,
            spins: RefCell::new(Vec::new()),
            stats,
        });

        let toolbar_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toolbar_row.style_context().add_class("clipnest-prefs-toolbar");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.set_halign(gtk::Align::Center);
        toolbar.set_hexpand(true);
        toolbar_row.pack_start(&toolbar, true, true, 0);

        let mut first: Option<gtk::RadioButton> = None;
        for &(id, title, icon) in PAGES {
            let page: gtk::Widget = match id {
                "general" => prefs.general_page(),
                "menu" => prefs.menu_page(),
                "type" => prefs.type_page(),
                "exclude" => prefs.exclude_page(),
                "shortcuts" => prefs.shortcuts_page(),
                _ => prefs.about_page(),
            };
            let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
            scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scroller.add(&page);
            prefs.stack.add_named(&scroller, id);

            let button = match &first {
                Some(first) => gtk::RadioButton::from_widget(first),
                None => gtk::RadioButton::new(),
            };
            button.set_mode(false);
            button.set_can_focus(false);
            button.style_context().add_class("clipnest-prefs-tab");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let image = gtk::Image::from_icon_name(Some(icon), gtk::IconSize::Dialog);
            image.set_pixel_size(28);
            content.pack_start(&image, false, false, 0);
            content.pack_start(&gtk::Label::new(Some(title)), false, false, 0);
            button.add(&content);
            let weak = Rc::downgrade(&prefs);
            button.connect_toggled(move |b| {
                if let (Some(p), true) = (weak.upgrade(), b.is_active()) {
                    p.show_page(id, title);
                }
            });
            toolbar.pack_start(&button, false, false, 0);
            first.get_or_insert(button);
        }

        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.pack_start(&toolbar_row, false, false, 0);
        outer.pack_start(&prefs.stack, true, true, 0);
        prefs.window.add(&outer);
        prefs
    }

    fn show_page(&self, id: &str, title: &str) {
        self.stack.set_visible_child_name(id);
        self.header.set_title(Some(title));
        self.window.set_title(title);
        self.window.set_focus(None::<&gtk::Widget>); // don't land in (and select) the first field
        for spin in self.spins.borrow().iter() {
            spin.select_region(0, 0);
        }
        if id == "about" {
            self.update_stats();
        }
    }

    // -- building blocks ---------------------------------------------------

    fn switch(&self, get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) -> gtk::Switch {
        let switch = gtk::Switch::new();
        switch.set_active(get(&self.host.config.get()));
        let host = self.host.clone();
        switch.connect_active_notify(move |s| {
            let value = s.is_active();
            host.config.update(|c| set(c, value));
        });
        switch
    }

    fn check(&self, label: &str, get: fn(&Settings) -> bool, set: fn(&mut Settings, bool)) -> gtk::CheckButton {
        let check = gtk::CheckButton::with_label(label);
        check.set_active(get(&self.host.config.get()));
        let host = self.host.clone();
        check.connect_toggled(move |c| {
            let value = c.is_active();
            host.config.update(|s| set(s, value));
        });
        check
    }

    fn spin(
        &self,
        low: f64,
        high: f64,
        step: f64,
        get: fn(&Settings) -> i64,
        set: fn(&mut Settings, i64),
    ) -> gtk::SpinButton {
        let spin = gtk::SpinButton::with_range(low, high, step);
        spin.set_value(get(&self.host.config.get()) as f64);
        let host = self.host.clone();
        spin.connect_value_changed(move |s| {
            let value = s.value_as_int() as i64;
            host.config.update(|c| set(c, value));
        });
        // GTK leaves the text selected after focus moves on; looks odd on other tabs.
        spin.connect_focus_out_event(|s, _| {
            s.select_region(0, 0);
            glib::Propagation::Proceed
        });
        self.spins.borrow_mut().push(spin.clone());
        spin
    }

    fn combo(&self, choices: &[(&str, &str)], current: &str, set: fn(&mut Settings, String)) -> gtk::ComboBoxText {
        let combo = gtk::ComboBoxText::new();
        for (id, label) in choices {
            combo.append(Some(id), label);
        }
        combo.set_active_id(Some(current));
        let host = self.host.clone();
        combo.connect_changed(move |c| {
            if let Some(id) = c.active_id() {
                host.config.update(|s| set(s, id.to_string()));
            }
        });
        combo
    }

    // -- pages --------------------------------------------------------------

    fn general_page(self: &Rc<Self>) -> gtk::Widget {
        let rows = Rows::new();
        let autostart = gtk::Switch::new();
        autostart.set_active(system::autostart_enabled());
        autostart.connect_active_notify(|s| system::set_autostart(s.is_active()));
        rows.row("Launch Clipnest at login", &autostart, None);

        let status = (self.host.paste_status)();
        rows.row(
            "Paste automatically after picking an item",
            &self.switch(|s| s.auto_paste, |s, v| s.auto_paste = v),
            Some(&status),
        );
        let cfg = self.host.config.get();
        rows.row(
            "Paste keystroke",
            &self.combo(PASTE_KEY_CHOICES, &cfg.paste_keys, |s, v| s.paste_keys = v),
            None,
        );
        rows.row(
            "Maximum history size",
            &self.spin(10.0, 5000.0, 10.0, |s| s.max_history, |s, v| s.max_history = v),
            Some("Pinned (★) items don't count towards the limit."),
        );
        rows.row(
            "Move a pasted item back to the top",
            &self.switch(|s| s.reorder_on_select, |s, v| s.reorder_on_select = v),
            None,
        );
        rows.row(
            "Ask before clearing history",
            &self.switch(|s| s.confirm_clear, |s, v| s.confirm_clear = v),
            None,
        );

        let clear = gtk::Button::with_label("Clear History…");
        let weak = Rc::downgrade(self);
        clear.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                (p.host.clear_history)(&p.window);
            }
        });
        rows.row("Pinned items are kept when clearing", &clear, None);
        rows.grid.upcast()
    }

    fn menu_page(self: &Rc<Self>) -> gtk::Widget {
        let rows = Rows::new();
        let cfg = self.host.config.get();
        let position = [
            ("pointer", "Text cursor (else mouse pointer)"),
            ("centre", "Centre of the screen"),
        ];
        rows.row(
            "Open the shortcut menu at",
            &self.combo(&position, &cfg.menu_position, |s, v| s.menu_position = v),
            Some(
                "Finding the text cursor needs the \"Clipnest helper\" GNOME Shell extension, which \
                       install.sh sets up (log out and back in once after installing).",
            ),
        );
        rows.row(
            "Items shown directly in the menu",
            &self.spin(0.0, 50.0, 1.0, |s| s.menu_inline_items, |s, v| s.menu_inline_items = v),
            None,
        );
        rows.row(
            "Items per folder",
            &self.spin(1.0, 100.0, 1.0, |s| s.menu_folder_size, |s, v| s.menu_folder_size = v),
            None,
        );
        rows.row(
            "Maximum items in the menu",
            &self.spin(0.0, 500.0, 1.0, |s| s.menu_max_items, |s, v| s.menu_max_items = v),
            None,
        );
        let chars = self.spin(5.0, 200.0, 1.0, |s| s.menu_title_length, |s, v| s.menu_title_length = v);
        rows.row("Characters per menu item", &chars, None);
        let fixed = self.switch(|s| s.menu_fixed_width, |s, v| s.menu_fixed_width = v);
        rows.row(
            "Fixed menu width",
            &fixed,
            Some(
                "Keeps the shortcut menu the same width every time; titles fill it instead of \
                       stopping at the character limit. (GNOME sizes the top-bar menu itself.)",
            ),
        );
        let width = self.spin(200.0, 1200.0, 10.0, |s| s.menu_width, |s, v| s.menu_width = v);
        rows.row("Menu width (pixels)", &width, None);
        let sync = {
            let (fixed, width, chars) = (fixed.clone(), width.clone(), chars.clone());
            move || {
                width.set_sensitive(fixed.is_active());
                chars.set_sensitive(!fixed.is_active());
            }
        };
        sync();
        fixed.connect_active_notify(move |_| sync());

        rows.row(
            "Mark menu items with numbers",
            &self.switch(|s| s.menu_show_numbers, |s, v| s.menu_show_numbers = v),
            None,
        );
        rows.row(
            "Show tooltips with the full text",
            &self.switch(|s| s.menu_show_tooltips, |s, v| s.menu_show_tooltips = v),
            None,
        );
        rows.row(
            "Show images in the menu",
            &self.switch(|s| s.menu_show_images, |s, v| s.menu_show_images = v),
            None,
        );
        let size = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        size.pack_start(
            &self.spin(16.0, 400.0, 1.0, |s| s.menu_thumb_width, |s, v| s.menu_thumb_width = v),
            false,
            false,
            0,
        );
        size.pack_start(&gtk::Label::new(Some("×")), false, false, 0);
        size.pack_start(
            &self.spin(
                16.0,
                400.0,
                1.0,
                |s| s.menu_thumb_height,
                |s, v| s.menu_thumb_height = v,
            ),
            false,
            false,
            0,
        );
        rows.row("Thumbnail size (width × height)", &size, None);
        rows.note(
            "Pick an item in the shortcut menu with 1…9 / 0 or the arrow keys. \
                   Panel icon: click for the menu, double-click for the search popup.",
        );
        rows.grid.upcast()
    }

    fn type_page(self: &Rc<Self>) -> gtk::Widget {
        // Laid out like Clipy's Type tab. RTFD and Universal Clipboard are
        // macOS-only, so they have no counterpart here.
        let grid = grid();
        grid.set_column_spacing(12);
        grid.set_halign(gtk::Align::Center);
        let heading = gtk::Label::new(Some("Save to History:"));
        heading.set_xalign(1.0);
        heading.set_valign(gtk::Align::Start);
        heading.set_margin_top(2);
        grid.attach(&heading, 0, 0, 1, 1);
        type Field = (&'static str, fn(&Settings) -> bool, fn(&mut Settings, bool));
        let types: [Field; 7] = [
            ("Plain Text", |s| s.store_text, |s, v| s.store_text = v),
            ("Rich Text Format (RTF)", |s| s.store_rtf, |s, v| s.store_rtf = v),
            ("PDF", |s| s.store_pdf, |s, v| s.store_pdf = v),
            ("Files", |s| s.store_files, |s, v| s.store_files = v),
            ("URL", |s| s.store_url, |s, v| s.store_url = v),
            ("Images", |s| s.store_images, |s, v| s.store_images = v),
            ("HTML", |s| s.store_html, |s, v| s.store_html = v),
        ];
        for (row, (label, get, set)) in types.iter().enumerate() {
            grid.attach(&self.check(label, *get, *set), 1, row as i32, 1, 1);
        }
        let row = types.len() as i32;
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.set_margin_top(14);
        separator.set_margin_bottom(14);
        grid.attach(&separator, 0, row, 2, 1);
        grid.attach(
            &self.check(
                "Do not save content marked as confidential to history",
                |s| s.ignore_secrets,
                |s, v| s.ignore_secrets = v,
            ),
            1,
            row + 1,
            1,
            1,
        );
        let hint = wrapped("Some apps, such as password managers, mark sensitive content as confidential.");
        hint.set_max_width_chars(50);
        hint.set_margin_start(26);
        dim(&hint);
        grid.attach(&hint, 1, row + 2, 1, 1);
        let note = wrapped(
            "Rich text, HTML, PDF and files are saved next to the plain text and pasted back \
                            with it; hold Shift when picking an item to paste plain text only. URL means \
                            copied text that is a single link.",
        );
        note.set_margin_top(18);
        dim(&note);
        grid.attach(&note, 0, row + 3, 2, 1);
        grid.upcast()
    }

    fn exclude_page(self: &Rc<Self>) -> gtk::Widget {
        let rows = Rows::new();
        rows.row(
            "Ignore whitespace-only copies",
            &self.switch(|s| s.ignore_blank, |s, v| s.ignore_blank = v),
            None,
        );
        rows.row(
            "Largest text to keep (KB)",
            &self.spin(1.0, 10240.0, 64.0, |s| s.max_text_kb, |s, v| s.max_text_kb = v),
            None,
        );
        let title = wrapped("Don't save text matching (one regular expression per line):");
        rows.grid.attach(&title, 0, rows.take(), 2, 1);
        let view = gtk::TextView::new();
        view.set_monospace(true);
        view.set_top_margin(6);
        view.set_bottom_margin(6);
        view.set_left_margin(6);
        view.set_right_margin(6);
        let buffer = view.buffer().expect("text view has a buffer");
        buffer.set_text(&self.host.config.get().ignore_patterns.join("\n"));
        let frame = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        frame.set_shadow_type(gtk::ShadowType::In);
        frame.set_min_content_height(90);
        frame.set_hexpand(true);
        frame.set_margin_top(4);
        frame.add(&view);
        rows.grid.attach(&frame, 0, rows.take(), 2, 1);
        let status = rows.note("");

        let save = {
            let host = self.host.clone();
            let buffer = buffer.clone();
            move || {
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .unwrap_or_default();
                let lines: Vec<String> = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .collect();
                let bad: Vec<String> = lines
                    .iter()
                    .filter_map(|l| {
                        Regex::new(l)
                            .err()
                            .map(|e| format!("{l}  ({})", e.to_string().lines().last().unwrap_or("")))
                    })
                    .collect();
                host.config.update(|s| s.ignore_patterns = lines);
                status.set_text(&if bad.is_empty() {
                    "Example: ^\\d{6}$ skips 6-digit one-time codes.".to_string()
                } else {
                    format!("Invalid, ignored: {}", bad.join("; "))
                });
            }
        };
        save();
        let save = Rc::new(save);
        let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
        buffer.connect_changed(move |_| {
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (save, slot) = (save.clone(), pending.clone());
            *pending.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(500), move || {
                slot.borrow_mut().take();
                save();
            }));
        });
        rows.note(
            "Wayland doesn't tell other apps which app copied something, so Clipnest filters by \
                   content instead of by app.",
        );
        rows.grid.upcast()
    }

    fn shortcuts_page(self: &Rc<Self>) -> gtk::Widget {
        let rows = Rows::new();
        let cfg = self.host.config.get();
        rows.row(
            "Clipboard history menu",
            &shortcut_button(self.host.clone(), "history", &cfg.hotkey_history),
            None,
        );
        rows.row(
            "Search history popup",
            &shortcut_button(self.host.clone(), "search", &cfg.hotkey_search),
            None,
        );
        let text = if system::gnome_shortcuts_supported() {
            "Saved as GNOME custom shortcuts (Settings → Keyboard → View and Customize Shortcuts → \
             Custom Shortcuts)."
                .to_string()
        } else {
            let launcher = system::launcher();
            format!(
                "Global shortcuts are set through your desktop. Bind these commands:\n  \
                     {launcher} history\n  {launcher} search"
            )
        };
        rows.note(&text);
        rows.grid.upcast()
    }

    fn about_page(self: &Rc<Self>) -> gtk::Widget {
        let page = gtk::Box::new(gtk::Orientation::Vertical, 8);
        page.set_border_width(32);
        page.set_halign(gtk::Align::Center);
        let icon = gtk::Image::from_icon_name(Some("clipnest"), gtk::IconSize::Dialog);
        icon.set_pixel_size(96);
        page.pack_start(&icon, false, false, 0);
        let name = gtk::Label::new(None);
        name.set_markup(&format!(
            "<span size='x-large' weight='bold'>Clipnest</span>  {}",
            env!("CARGO_PKG_VERSION")
        ));
        page.pack_start(&name, false, false, 0);
        let tagline = gtk::Label::new(None);
        tagline.set_markup("<i>Your clipboard, remembered.</i>");
        page.pack_start(&tagline, false, false, 0);
        let about = gtk::Label::new(Some(
            "Clipnest keeps everything you copy (text, links, images, files and formatted text) and \
             gives it back with one keystroke: press Super+Shift+V and the menu opens right at your \
             cursor, ready to paste. Your history stays on this computer.",
        ));
        about.set_line_wrap(true);
        about.set_max_width_chars(56);
        about.set_justify(gtk::Justification::Center);
        about.set_margin_top(6);
        page.pack_start(&about, false, false, 0);
        let maker = gtk::Label::new(None);
        maker.set_markup("Made by <b>erag</b> · <a href=\"https://erag.in\">erag.in</a>");
        maker.set_margin_top(8);
        page.pack_start(&maker, false, false, 0);
        let copyright = gtk::Label::new(Some("© 2026 erag · MIT License"));
        dim(&copyright);
        page.pack_start(&copyright, false, false, 0);
        page.pack_start(&self.stats, false, false, 6);
        self.update_stats();
        let dir = self.host.data_dir.clone();
        let location = gtk::Label::new(Some(&format!("Data: {}", dir.display())));
        location.set_line_wrap(true);
        location.set_justify(gtk::Justification::Center);
        dim(&location);
        page.pack_start(&location, false, false, 0);
        let open = gtk::Button::with_label("Open Data Folder");
        open.set_halign(gtk::Align::Center);
        open.set_margin_top(6);
        let weak = Rc::downgrade(self);
        open.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                let uri = gio::File::for_path(&dir).uri();
                let _ = gtk::show_uri_on_window(Some(&p.window), &uri, gtk::current_event_time());
            }
        });
        page.pack_start(&open, false, false, 0);
        let update = gtk::Label::new(Some("To update, install the newest release from GitHub."));
        update.set_margin_top(12);
        dim(&update);
        page.pack_start(&update, false, false, 0);
        page.upcast()
    }

    fn update_stats(&self) {
        let count = (self.host.history_count)().max(0) as usize;
        self.stats.set_text(&format!("{} items in history", thousands(count)));
    }
}

fn accel_label(binding: &str) -> String {
    if binding.is_empty() {
        return "Disabled".into();
    }
    let (key, mods) = gtk::accelerator_parse(binding);
    if key == 0 {
        binding.into()
    } else {
        gtk::accelerator_get_label(key, mods)
            .map(|s| s.to_string())
            .unwrap_or_else(|| binding.into())
    }
}

/// A button that records a key combination: click it, then press the keys.
fn shortcut_button(host: Rc<PrefsHost>, which: &'static str, current: &str) -> gtk::Button {
    let button = gtk::Button::with_label(&accel_label(current));
    button.set_size_request(200, -1);
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::new(RefCell::new(None));
    button.connect_clicked(move |button| {
        if handler.borrow().is_some() {
            return;
        }
        let Some(window) = button.toplevel().and_then(|t| t.downcast::<gtk::Window>().ok()) else {
            return;
        };
        button.set_label("Press keys…  (Esc cancel, ⌫ clear)");
        let (host, button_ref, slot, win) = (host.clone(), button.clone(), handler.clone(), window.clone());
        let id = window.connect_key_press_event(move |_, event| {
            let finish = |binding: Option<String>| {
                if let Some(binding) = &binding {
                    (host.set_hotkey)(which, binding);
                }
                let cfg = host.config.get();
                let current = if which == "history" {
                    cfg.hotkey_history
                } else {
                    cfg.hotkey_search
                };
                button_ref.set_label(&accel_label(&current));
                if let Some(id) = slot.borrow_mut().take() {
                    win.disconnect(id);
                }
            };
            record_key(event, &button_ref, finish)
        });
        *handler.borrow_mut() = Some(id);
    });
    button
}

fn record_key(event: &gdk::EventKey, button: &gtk::Button, finish: impl FnOnce(Option<String>)) -> glib::Propagation {
    use gdk::keys::constants as key;
    let keyval = event.keyval().to_lower();
    let modifier_keys = [
        key::Shift_L,
        key::Shift_R,
        key::Control_L,
        key::Control_R,
        key::Alt_L,
        key::Alt_R,
        key::Super_L,
        key::Super_R,
        key::Meta_L,
        key::Meta_R,
        key::ISO_Level3_Shift,
    ];
    if modifier_keys.contains(&keyval) {
        return glib::Propagation::Stop;
    }
    let mut state = event.state();
    if let Some(keymap) = gdk::Keymap::for_display(&button.display()) {
        keymap.add_virtual_modifiers(&mut state);
    }
    let mut mods = state & gtk::accelerator_get_default_mod_mask();
    if mods.contains(gdk::ModifierType::SUPER_MASK) {
        // Mod4 maps to Super *and* Hyper on most layouts; keep only Super.
        mods -= gdk::ModifierType::HYPER_MASK | gdk::ModifierType::META_MASK;
    }
    if keyval == key::Escape && mods.is_empty() {
        finish(None);
    } else if keyval == key::BackSpace && mods.is_empty() {
        finish(Some(String::new()));
    } else if mods.is_empty() && !(*key::F1..=*key::F35).contains(&*keyval) {
        button.set_label("Use a modifier (Ctrl / Alt / Super)…");
    } else {
        finish(gtk::accelerator_name(*keyval, mods).map(|s| s.to_string()));
    }
    glib::Propagation::Stop
}
