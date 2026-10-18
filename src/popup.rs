//! The search popup: the clipboard history, filtered as you type.
//!
//! Keys: type to filter · ↑/↓ select · Enter paste · Shift+Enter plain text ·
//! Ctrl+Enter copy only · Alt+1…9, Alt+0 quick pick · Ctrl+P pin ·
//! Shift+Delete delete · Esc close.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

use crate::storage::HistoryItem;
use crate::util::{describe_text, now, present, relative_time, summarize, thumbnail};

const MAX_ROWS: usize = 400;

const CSS: &str = "
.clipnest-window { background-color: transparent; }
.clipnest-frame {
    background-color: @theme_base_color;
    border: 1px solid alpha(@theme_fg_color, 0.18);
    border-radius: 12px;
}
.clipnest-header { padding: 10px 12px 6px 12px; }
.clipnest-search { font-size: 1.15em; min-height: 34px; }
.clipnest-frame list { background-color: transparent; }
.clipnest-frame list row { border-radius: 8px; margin: 0 6px; }
.clipnest-frame list row:selected { background-color: @theme_selected_bg_color; }
.clipnest-frame list row:selected label { color: @theme_selected_fg_color; }
.clipnest-number { opacity: 0.55; font-weight: bold; }
.clipnest-subtitle { font-size: 0.85em; opacity: 0.65; }
.clipnest-placeholder { opacity: 0.6; padding: 48px 24px; }
.clipnest-footer {
    font-size: 0.8em; opacity: 0.65; padding: 6px 14px 8px 14px;
    border-top: 1px solid alpha(@theme_fg_color, 0.1);
}
";

/// What the popup needs from the rest of the app.
pub struct PopupHost {
    pub history: Box<dyn Fn() -> Vec<HistoryItem>>,
    pub image_path: Box<dyn Fn(&HistoryItem) -> std::path::PathBuf>,
    pub can_auto_paste: Box<dyn Fn() -> bool>,
    /// (item id, paste, plain)
    pub use_item: Box<dyn Fn(i64, bool, bool)>,
    pub set_pinned: Box<dyn Fn(i64, bool)>,
    pub delete: Box<dyn Fn(i64)>,
    pub changed: Box<dyn Fn()>,
    pub preferences: Box<dyn Fn()>,
}

pub struct Popup {
    pub window: gtk::Window,
    host: PopupHost,
    search: gtk::SearchEntry,
    listbox: gtk::ListBox,
    scroller: gtk::ScrolledWindow,
    placeholder: gtk::Label,
    footer: gtk::Label,
    rows: RefCell<Vec<(gtk::ListBoxRow, HistoryItem)>>,
    was_active: Cell<bool>,
}

fn styled<W: IsA<gtk::Widget>>(widget: W, class: &str) -> W {
    widget.style_context().add_class(class);
    widget
}

impl Popup {
    pub fn new(app: &gtk::Application, host: PopupHost) -> Rc<Popup> {
        let window = gtk::Window::builder().application(app).title("Clipnest").build();
        window.set_decorated(false);
        window.set_skip_taskbar_hint(true);
        window.set_skip_pager_hint(true);
        window.set_keep_above(true);
        window.set_resizable(false);
        window.set_position(gtk::WindowPosition::Center);
        window.set_size_request(640, 500);
        if let Some(screen) = GtkWindowExt::screen(&window) {
            if screen.is_composited() {
                if let Some(visual) = screen.rgba_visual() {
                    window.set_visual(Some(&visual)); // lets the frame have rounded corners
                }
            }
            let provider = gtk::CssProvider::new();
            let _ = provider.load_from_data(CSS.as_bytes());
            gtk::StyleContext::add_provider_for_screen(&screen, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
        }
        window.style_context().add_class("clipnest-window");

        let frame = styled(gtk::Box::new(gtk::Orientation::Vertical, 0), "clipnest-frame");
        window.add(&frame);

        let header = styled(gtk::Box::new(gtk::Orientation::Horizontal, 8), "clipnest-header");
        let search = styled(gtk::SearchEntry::new(), "clipnest-search");
        search.set_placeholder_text(Some("Search clipboard history…"));
        header.pack_start(&search, true, true, 0);

        let model = gio::Menu::new();
        model.append(Some("Preferences…"), Some("app.preferences"));
        model.append(Some("Clear History…"), Some("app.clear"));
        model.append(Some("Quit Clipnest"), Some("app.quit"));
        // A popover stays inside this window, so opening it doesn't count as losing focus.
        let menu_button = gtk::MenuButton::new();
        menu_button.set_menu_model(Some(&model));
        menu_button.set_use_popover(true);
        menu_button.set_can_focus(false);
        menu_button.set_relief(gtk::ReliefStyle::None);
        menu_button.add(&gtk::Image::from_icon_name(
            Some("open-menu-symbolic"),
            gtk::IconSize::Button,
        ));
        header.pack_start(&menu_button, false, false, 0);

        let listbox = gtk::ListBox::new();
        listbox.set_selection_mode(gtk::SelectionMode::Single);
        listbox.set_activate_on_single_click(true);
        let placeholder = styled(gtk::Label::new(None), "clipnest-placeholder");
        placeholder.set_justify(gtk::Justification::Center);
        placeholder.show();
        listbox.set_placeholder(Some(&placeholder));
        let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        scroller.add(&listbox);

        let footer = styled(gtk::Label::new(None), "clipnest-footer");
        footer.set_xalign(0.0);
        footer.set_ellipsize(pango::EllipsizeMode::End);

        frame.pack_start(&header, false, false, 0);
        frame.pack_start(&scroller, true, true, 0);
        frame.pack_start(&footer, false, false, 0);

        let popup = Rc::new(Popup {
            window,
            host,
            search,
            listbox,
            scroller,
            placeholder,
            footer,
            rows: RefCell::new(Vec::new()),
            was_active: Cell::new(false),
        });

        let weak = Rc::downgrade(&popup);
        popup.search.connect_search_changed(move |_| {
            if let Some(p) = weak.upgrade() {
                p.refresh();
            }
        });
        let weak = Rc::downgrade(&popup);
        popup.listbox.connect_row_activated(move |_, row| {
            if let Some(p) = weak.upgrade() {
                p.activate_row(Some(row.clone()), true, false);
            }
        });
        popup.window.connect_delete_event(|window, _| {
            window.hide();
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&popup);
        popup
            .window
            .connect_key_press_event(move |_, event| match weak.upgrade() {
                Some(p) => p.on_key(event),
                None => glib::Propagation::Proceed,
            });
        let weak = Rc::downgrade(&popup);
        popup.window.connect_focus_in_event(move |_, _| {
            if let Some(p) = weak.upgrade() {
                p.was_active.set(true);
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(&popup);
        popup.window.connect_focus_out_event(move |_, _| {
            // Behave like a menu: close when the user clicks elsewhere.
            if let Some(p) = weak.upgrade() {
                if p.was_active.get() {
                    let weak = Rc::downgrade(&p);
                    glib::timeout_add_local_once(Duration::from_millis(120), move || {
                        if let Some(p) = weak.upgrade() {
                            if p.window.is_visible() && !p.window.is_active() {
                                p.window.hide();
                            }
                        }
                    });
                }
            }
            glib::Propagation::Proceed
        });
        frame.show_all();
        popup.window.realize();
        popup
    }

    pub fn toggle(&self) {
        if self.window.is_visible() {
            self.window.hide();
        } else {
            self.open();
        }
    }

    pub fn open(&self) {
        self.search.set_text("");
        self.refresh();
        self.was_active.set(false);
        present(&self.window);
        self.search.grab_focus();
    }

    pub fn hide(&self) {
        self.window.hide();
    }

    fn number(index: usize) -> String {
        if index < 10 {
            ((index + 1) % 10).to_string()
        } else {
            String::new()
        }
    }

    fn make_row(&self, entry: &HistoryItem, index: usize) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.set_can_focus(false); // keep keyboard focus in the search entry
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        hbox.set_margin_start(8);
        hbox.set_margin_end(10);
        hbox.set_margin_top(5);
        hbox.set_margin_bottom(5);
        let num = styled(gtk::Label::new(Some(&Self::number(index))), "clipnest-number");
        num.set_xalign(1.0);
        num.set_width_chars(2);
        hbox.pack_start(&num, false, false, 0);

        let when = relative_time(entry.created, now());
        let (title, subtitle) = if entry.is_image() {
            let path = (self.host.image_path)(entry);
            if let Some(pixbuf) = thumbnail(&path, 40) {
                hbox.pack_start(&gtk::Image::from_pixbuf(Some(&pixbuf)), false, false, 0);
            }
            row.set_has_tooltip(true);
            row.connect_query_tooltip(move |_, _, _, _, tooltip| match thumbnail(&path, 320) {
                Some(big) => {
                    tooltip.set_icon(Some(&big));
                    true
                }
                None => false,
            });
            (entry.summary.clone(), when)
        } else {
            let full = if entry.content.is_empty() {
                &entry.summary
            } else {
                &entry.content
            };
            let tooltip = if full.chars().count() <= 1500 {
                full.clone()
            } else {
                format!("{}\n…", full.chars().take(1500).collect::<String>())
            };
            row.set_tooltip_text(Some(&tooltip));
            let detail = match entry.kind.as_str() {
                "files" if entry.content.contains('\n') => "files".to_string(),
                "files" => "file".to_string(),
                "data" => entry.summary.clone(),
                _ => describe_text(&entry.content),
            };
            (summarize(entry.display_text(), 140), format!("{when} · {detail}"))
        };

        let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
        text.set_valign(gtk::Align::Center);
        let title_label = gtk::Label::new(Some(&title));
        title_label.set_xalign(0.0);
        title_label.set_single_line_mode(true);
        title_label.set_ellipsize(pango::EllipsizeMode::End);
        text.pack_start(&title_label, false, false, 0);
        let sub = styled(gtk::Label::new(Some(&subtitle)), "clipnest-subtitle");
        sub.set_xalign(0.0);
        sub.set_single_line_mode(true);
        sub.set_ellipsize(pango::EllipsizeMode::End);
        text.pack_start(&sub, false, false, 0);
        hbox.pack_start(&text, true, true, 0);
        if entry.pinned {
            hbox.pack_end(
                &gtk::Image::from_icon_name(Some("starred-symbolic"), gtk::IconSize::Menu),
                false,
                false,
                0,
            );
        }
        row.add(&hbox);
        row
    }

    pub fn refresh(&self) {
        for child in self.listbox.children() {
            self.listbox.remove(&child);
        }
        let query = self.search.text().to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let mut rows = Vec::new();
        for entry in (self.host.history)() {
            let haystack = if entry.is_image() {
                format!("{} image", entry.summary).to_lowercase()
            } else {
                format!("{}\n{}", entry.display_text(), entry.content).to_lowercase()
            };
            if !words.iter().all(|w| haystack.contains(w)) {
                continue;
            }
            if rows.len() >= MAX_ROWS {
                break;
            }
            let row = self.make_row(&entry, rows.len());
            self.listbox.add(&row);
            rows.push((row, entry));
        }
        self.placeholder.set_text(if words.is_empty() {
            "Nothing copied yet.\nCopy some text or an image and it shows up here."
        } else {
            "No matches."
        });
        self.listbox.show_all();
        let first = rows.first().map(|(row, _)| row.clone());
        *self.rows.borrow_mut() = rows;
        if let Some(first) = first {
            self.listbox.select_row(Some(&first));
            self.scroll_to(&first);
        }
        let paste = if (self.host.can_auto_paste)() {
            "Enter paste"
        } else {
            "Enter copy (then Ctrl+V)"
        };
        self.footer.set_text(
            &[
                paste,
                "Shift+Enter plain text",
                "Ctrl+Enter copy",
                "Alt+1…0 pick",
                "Ctrl+P pin",
                "Shift+Del delete",
                "Esc close",
            ]
            .join("   ·   "),
        );
    }

    fn selected_index(&self) -> Option<usize> {
        let selected = self.listbox.selected_row()?;
        self.rows.borrow().iter().position(|(row, _)| row == &selected)
    }

    fn select_index(&self, index: usize) {
        let row = self.rows.borrow().get(index).map(|(row, _)| row.clone());
        if let Some(row) = row {
            self.listbox.select_row(Some(&row));
            self.scroll_to(&row);
        }
    }

    fn move_by(&self, delta: i32) {
        let count = self.rows.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self.selected_index().map(|i| i as i32).unwrap_or(-1);
        self.select_index((current + delta).clamp(0, count - 1) as usize);
    }

    /// Runs after layout so the row has an allocation; by then a refresh may
    /// already have replaced it.
    fn scroll_to(&self, row: &gtk::ListBoxRow) {
        let row = row.clone();
        let scroller = self.scroller.clone();
        glib::idle_add_local_once(move || {
            if row.parent().is_none() || !scroller.is_realized() {
                return;
            }
            let alloc = row.allocation();
            let adj = scroller.vadjustment();
            let (y, h) = (alloc.y() as f64, alloc.height() as f64);
            if y < adj.value() {
                adj.set_value(y);
            } else if y + h > adj.value() + adj.page_size() {
                adj.set_value(y + h - adj.page_size());
            }
        });
    }

    fn activate_row(&self, row: Option<gtk::ListBoxRow>, paste: bool, plain: bool) {
        let Some(row) = row else { return };
        let id = self.rows.borrow().iter().find(|(r, _)| r == &row).map(|(_, e)| e.id);
        if let Some(id) = id {
            self.window.hide();
            (self.host.use_item)(id, paste, plain);
        }
    }

    fn refresh_keeping(&self, id: Option<i64>, index: Option<usize>) {
        self.refresh();
        let count = self.rows.borrow().len();
        let target = match (id, index) {
            (Some(id), _) => self.rows.borrow().iter().position(|(_, e)| e.id == id),
            (None, Some(index)) if count > 0 => Some(index.min(count - 1)),
            _ => None,
        };
        if let Some(target) = target {
            self.select_index(target);
        }
    }

    /// Called when history changed while the popup may be open.
    pub fn history_changed(&self) {
        if self.window.is_visible() {
            let id = self
                .selected_index()
                .and_then(|i| self.rows.borrow().get(i).map(|(_, e)| e.id));
            self.refresh_keeping(id, None);
        }
    }

    fn on_key(&self, event: &gdk::EventKey) -> glib::Propagation {
        use gdk::keys::constants as key;
        let keyval = event.keyval();
        let mods = event.state() & gtk::accelerator_get_default_mod_mask();
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = mods.contains(gdk::ModifierType::MOD1_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let empty_search = self.search.text().is_empty();
        let stop = glib::Propagation::Stop;

        if keyval == key::Escape {
            if empty_search {
                self.window.hide();
            } else {
                self.search.set_text("");
            }
            return stop;
        }
        if keyval == key::Down || keyval == key::KP_Down || (ctrl && keyval == key::n) {
            self.move_by(1);
            return stop;
        }
        if keyval == key::Up || keyval == key::KP_Up {
            self.move_by(-1);
            return stop;
        }
        if keyval == key::Page_Down || keyval == key::KP_Page_Down {
            self.move_by(8);
            return stop;
        }
        if keyval == key::Page_Up || keyval == key::KP_Page_Up {
            self.move_by(-8);
            return stop;
        }
        if keyval == key::Return || keyval == key::KP_Enter || keyval == key::ISO_Enter {
            // Shift: paste just the plain text, without formatting
            self.activate_row(self.listbox.selected_row(), !ctrl, shift);
            return stop;
        }
        if alt {
            if let Some(digit) = keyval.to_unicode().and_then(|c| c.to_digit(10)) {
                let index = (digit as usize + 9) % 10;
                let row = self.rows.borrow().get(index).map(|(row, _)| row.clone());
                if row.is_some() {
                    self.activate_row(row, true, false);
                }
                return stop;
            }
        }
        if ctrl && keyval == key::comma {
            self.window.hide();
            (self.host.preferences)();
            return stop;
        }
        let Some(index) = self.selected_index() else {
            return glib::Propagation::Proceed;
        };
        let entry = self.rows.borrow()[index].1.clone();
        if ctrl && keyval == key::p {
            (self.host.set_pinned)(entry.id, !entry.pinned);
            self.refresh_keeping(Some(entry.id), None);
            (self.host.changed)();
            return stop;
        }
        if (keyval == key::Delete || keyval == key::KP_Delete) && (shift || empty_search) {
            (self.host.delete)(entry.id);
            (self.host.changed)();
            self.refresh_keeping(None, Some(index));
            return stop;
        }
        glib::Propagation::Proceed
    }
}
