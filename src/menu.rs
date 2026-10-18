//! The history menu.
//!
//! One model (`build_menu`) is rendered two ways: as a GTK popup menu for the
//! hotkey (`MenuPresenter`: opens at the mouse pointer, thumbnails, folder
//! icons, flyout submenus, 1…9/0 keys, an icon row for the actions, optional
//! fixed width) and in the top bar via tray.rs.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glib::translate::{ToGlibPtr, ToGlibPtrMut};
use gtk::prelude::*;

use crate::config::Settings;
use crate::storage::HistoryItem;
use crate::util::{present, thumbnail_box};

const SHORTEN: &str = "...";
/// Room a menu row needs besides its label (item padding, submenu arrow), used
/// to size labels when the menu has a fixed width.
const ROW_CHROME: i32 = 44;

pub type Callback = Rc<dyn Fn()>;

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,          // full text, e.g. "3. (Image)"
    pub text: String,           // label without the number
    pub number: Option<String>, // "3": shown in its own column by the GTK menu
    pub key: Option<char>,      // '1'…'9', '0': picks the item in the GTK menu
    pub callback: Option<Callback>,
    pub plain_callback: Option<Callback>, // used instead when Shift is held
    pub separator: bool,
    pub enabled: bool,
    pub children: Vec<MenuItem>,
    pub image_path: Option<PathBuf>,
    pub folder: bool,
    pub tooltip: Option<String>,
    pub icon_name: Option<String>,
    pub toolbar: bool, // GTK menu: icon-only, in one row at the bottom
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> MenuItem {
        let label = label.into();
        MenuItem {
            text: label.clone(),
            label,
            number: None,
            key: None,
            callback: None,
            plain_callback: None,
            separator: false,
            enabled: true,
            children: Vec::new(),
            image_path: None,
            folder: false,
            tooltip: None,
            icon_name: None,
            toolbar: false,
        }
    }

    pub fn separator() -> MenuItem {
        MenuItem {
            separator: true,
            ..MenuItem::new("")
        }
    }

    fn disabled(label: &str) -> MenuItem {
        MenuItem {
            enabled: false,
            ..MenuItem::new(label)
        }
    }

    fn action(label: &str, icon: &str, callback: Callback) -> MenuItem {
        MenuItem {
            callback: Some(callback),
            icon_name: Some(icon.into()),
            toolbar: true,
            ..MenuItem::new(label)
        }
    }
}

/// What the menu's entries do.
pub struct MenuActions {
    /// (item id, from the top bar, plain text only)
    pub use_item: Rc<dyn Fn(i64, bool, bool)>,
    pub search: Callback,
    pub clear: Callback,
    pub preferences: Callback,
    pub quit: Callback,
    /// Show a folder's entries as their own menu (used by the top bar).
    pub open_folder: Rc<dyn Fn(Vec<MenuItem>)>,
}

/// First line, trimmed, cut to `length` with a trailing "...".
pub fn item_title(text: &str, length: usize) -> String {
    let line = text.trim().lines().next().unwrap_or("");
    if line.is_empty() {
        return "(whitespace)".into();
    }
    let length = length.max(SHORTEN.len());
    if line.chars().count() > length {
        let cut: String = line.chars().take(length - SHORTEN.len()).collect();
        format!("{cut}{SHORTEN}")
    } else {
        line.to_string()
    }
}

fn history_item(
    cfg: &Settings,
    index: usize,
    entry: &HistoryItem,
    image_path: &dyn Fn(&HistoryItem) -> PathBuf,
    actions: &MenuActions,
    from_panel: bool,
    title_length: usize,
) -> MenuItem {
    let (mut title, image, tooltip) = if entry.is_image() {
        (
            "(Image)".to_string(),
            Some(image_path(entry)),
            Some(entry.summary.clone()),
        )
    } else {
        let full = if entry.content.is_empty() {
            &entry.summary
        } else {
            &entry.content
        };
        let tooltip = if full.chars().count() <= 600 {
            full.clone()
        } else {
            format!("{}…", full.chars().take(600).collect::<String>())
        };
        (item_title(entry.display_text(), title_length), None, Some(tooltip))
    };
    if entry.pinned {
        title = format!("★ {title}");
    }
    let number = index + 1;
    let (label, number_text, key) = if cfg.menu_show_numbers {
        let key = (number <= 10).then(|| char::from_digit((number % 10) as u32, 10).unwrap());
        (format!("{number}. {title}"), Some(number.to_string()), key)
    } else {
        (title.clone(), None, None)
    };
    let id = entry.id;
    let use_item = actions.use_item.clone();
    let use_plain = actions.use_item.clone();
    MenuItem {
        text: title,
        number: number_text,
        key,
        callback: Some(Rc::new(move || use_item(id, from_panel, false))),
        plain_callback: Some(Rc::new(move || use_plain(id, from_panel, true))),
        image_path: image,
        tooltip: if cfg.menu_show_tooltips { tooltip } else { None },
        ..MenuItem::new(label)
    }
}

/// (entries listed directly, entries per folder) so that the history takes at
/// most `max_rows` rows. Fewer entries are listed directly first; if folders
/// of the configured size still don't fit (small folders save no rows), the
/// folders grow instead.
pub fn fit_layout(total: usize, inline: usize, folder_size: usize, max_rows: usize) -> (usize, usize) {
    let rows = |inline: usize, size: usize| inline + (total - inline).div_ceil(size);
    let max_rows = max_rows.max(1);
    let mut fit = inline.min(total);
    loop {
        if rows(fit, folder_size) <= max_rows {
            return (fit, folder_size);
        }
        if fit == 0 {
            break;
        }
        fit -= 1;
    }
    let fit = inline.min(total).min(max_rows - 1);
    (fit, folder_size.max((total - fit).div_ceil(max_rows - fit)))
}

/// The menu: history (newest inline, older ones in folders), then the actions.
///
/// `from_panel`: chosen from the top bar, which needs a little longer to hand
/// focus back before pasting. `title_length` overrides the configured limit.
/// `max_rows` caps the history rows (inline entries + folders) so the menu
/// fits on screen; what doesn't fit moves into the folders.
pub fn build_menu(
    cfg: &Settings,
    entries: &[HistoryItem],
    image_path: &dyn Fn(&HistoryItem) -> PathBuf,
    actions: &MenuActions,
    from_panel: bool,
    title_length: Option<usize>,
    max_rows: Option<usize>,
) -> Vec<MenuItem> {
    let title_length = title_length.unwrap_or(cfg.menu_title_length.max(1) as usize);
    let mut size = cfg.menu_folder_size.max(1) as usize;
    let mut inline = cfg.menu_inline_items.max(0) as usize;
    if let Some(max_rows) = max_rows {
        (inline, size) = fit_layout(entries.len(), inline, size, max_rows);
    }
    let mut items = vec![MenuItem::disabled("History")];
    for (index, entry) in entries.iter().take(inline).enumerate() {
        items.push(history_item(
            cfg,
            index,
            entry,
            image_path,
            actions,
            from_panel,
            title_length,
        ));
    }
    let mut start = inline;
    while start < entries.len() {
        let chunk = &entries[start..(start + size).min(entries.len())];
        let children: Vec<MenuItem> = chunk
            .iter()
            .enumerate()
            .map(|(i, e)| history_item(cfg, start + i, e, image_path, actions, from_panel, title_length))
            .collect();
        let label = format!("{} - {}", start + 1, start + chunk.len());
        if from_panel {
            // GNOME's top-bar menu opens submenus inline, squeezed into a small
            // scrolled box; open the folder as a menu of its own instead.
            let open = actions.open_folder.clone();
            items.push(MenuItem {
                icon_name: Some("go-next-symbolic".into()),
                callback: Some(Rc::new(move || open(children.clone()))),
                ..MenuItem::new(label)
            });
        } else {
            items.push(MenuItem {
                folder: true,
                children,
                ..MenuItem::new(label)
            });
        }
        start += size;
    }
    if entries.is_empty() {
        items.push(MenuItem::disabled("(nothing copied yet)"));
    }
    items.push(MenuItem::separator());
    items.push(MenuItem::action(
        "Search History...",
        "system-search-symbolic",
        actions.search.clone(),
    ));
    items.push(MenuItem::action(
        "Clear History",
        "edit-clear-all-symbolic",
        actions.clear.clone(),
    ));
    items.push(MenuItem::action(
        "Preferences...",
        "emblem-system-symbolic",
        actions.preferences.clone(),
    ));
    items.push(MenuItem::action(
        "Quit Clipnest",
        "system-shutdown-symbolic",
        actions.quit.clone(),
    ));
    items
}

// ---------------------------------------------------------------------------
// GTK popup menu
// ---------------------------------------------------------------------------

/// Where the menu opens.
#[derive(Clone, Copy)]
enum Placement {
    /// Below the text cursor, where the paste will go; else at the pointer.
    Cursor,
    /// At the mouse pointer.
    Pointer,
    Centre,
    At(i32, i32),
}

/// The "Clipnest helper" GNOME Shell extension (extension/) can tell where the
/// text cursor and the pointer are; Wayland hides both from apps.
fn ask_shell(method: &str) -> Option<glib::Variant> {
    let bus = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>).ok()?;
    bus.call_sync(
        Some("in.erag.Clipnest.Shell"),
        "/in/erag/Clipnest/Shell",
        "in.erag.Clipnest.Shell",
        method,
        None,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        200,
        None::<&gio::Cancellable>,
    )
    .ok()
}

fn shell_caret() -> Option<gdk::Rectangle> {
    let (found, x, y, width, height) = ask_shell("GetCaret")?.get::<(bool, i32, i32, i32, i32)>()?;
    found.then(|| gdk::Rectangle::new(x, y, width, height))
}

fn shell_pointer() -> Option<(i32, i32)> {
    ask_shell("GetPointer")?.get::<(i32, i32)>()
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Closed,
    Waiting, // for focus and the pointer position
    Open,
}

/// Pops the history menu up as a real GTK menu.
///
/// A GTK menu needs a focused window of ours to hold the keyboard grab, so a
/// 1×1 invisible anchor window is focused first and the menu is attached to it.
///
/// Wayland doesn't tell apps where the mouse is, and XWayland only learns it
/// while the pointer is over an X11 window. So to open the menu at the mouse,
/// an invisible window covering the screen is mapped for a
/// moment: the compositor then reports the pointer to it, and it's unmapped
/// again before the menu appears.
///
/// Focus is tracked through the window manager's _NET_ACTIVE_WINDOW: GTK's own
/// focus state turns false as soon as the menu grabs the keyboard, and an X
/// grab can't see clicks on Wayland windows, so this is how we notice the user
/// moving on (the menu then closes instead of being stuck on screen).
pub struct MenuPresenter {
    settings: Box<dyn Fn() -> Settings>,
    build: Box<dyn Fn(Option<usize>) -> Vec<MenuItem>>,
    anchor: gtk::Window,
    probe: gtk::Window,
    menu: RefCell<Option<gtk::Menu>>,
    state: Cell<State>,
    located: Cell<bool>,
    poll_id: RefCell<Option<glib::SourceId>>,
    locate_id: RefCell<Option<glib::SourceId>>,
    started: Cell<Instant>,
    retried: Cell<bool>,
    grab_tries: Cell<u32>,
    width: Cell<i32>,
    keys: RefCell<HashMap<char, gtk::MenuItem>>,
}

const POLL: Duration = Duration::from_millis(30);
const FOCUS_TIMEOUT: Duration = Duration::from_millis(900); // wait for GNOME to focus the anchor
const LOCATE: Duration = Duration::from_millis(120); // longest wait for the real pointer position

impl MenuPresenter {
    pub fn new(
        app: &gtk::Application,
        settings: impl Fn() -> Settings + 'static,
        build: impl Fn(Option<usize>) -> Vec<MenuItem> + 'static,
    ) -> Rc<MenuPresenter> {
        let anchor = gtk::Window::builder().application(app).title("Clipnest").build();
        anchor.set_decorated(false);
        anchor.set_skip_taskbar_hint(true);
        anchor.set_skip_pager_hint(true);
        anchor.set_keep_above(true);
        anchor.set_default_size(1, 1);
        WidgetExt::set_opacity(&anchor, 0.0);

        let probe = gtk::Window::new(gtk::WindowType::Popup); // finds the pointer
        if let Some(visual) = GtkWindowExt::screen(&probe).and_then(|s| s.rgba_visual()) {
            probe.set_visual(Some(&visual));
        }
        WidgetExt::set_opacity(&probe, 0.0);
        probe.add_events(gdk::EventMask::POINTER_MOTION_MASK);

        let presenter = Rc::new(MenuPresenter {
            settings: Box::new(settings),
            build: Box::new(build),
            anchor,
            probe,
            menu: RefCell::new(None),
            state: Cell::new(State::Closed),
            located: Cell::new(true),
            poll_id: RefCell::new(None),
            locate_id: RefCell::new(None),
            started: Cell::new(Instant::now()),
            retried: Cell::new(false),
            grab_tries: Cell::new(0),
            width: Cell::new(0),
            keys: RefCell::new(HashMap::new()),
        });
        let weak = Rc::downgrade(&presenter);
        presenter.probe.connect_motion_notify_event(move |_, _| {
            if let Some(p) = weak.upgrade() {
                p.on_probe_motion();
            }
            glib::Propagation::Proceed
        });
        presenter
    }

    pub fn toggle(self: &Rc<Self>) {
        if self.state.get() != State::Closed {
            self.close();
            return;
        }
        let cfg = (self.settings)();
        // A fixed-width menu fills its width with text instead of cutting
        // titles at the character limit; GTK ellipsizes whatever doesn't fit.
        self.width
            .set(if cfg.menu_fixed_width { cfg.menu_width as i32 } else { 0 });
        let items = (self.build)(if self.width.get() > 0 { Some(300) } else { None });
        self.open(
            &items,
            if cfg.menu_position == "pointer" {
                Placement::Cursor
            } else {
                Placement::Centre
            },
        );
    }

    /// The menu for a click on the top-bar icon: right below the icon (or at
    /// the pointer when the tray doesn't say where the icon is).
    pub fn toggle_below(self: &Rc<Self>, icon: Option<gdk::Rectangle>) {
        if self.state.get() != State::Closed {
            self.close();
            return;
        }
        let cfg = (self.settings)();
        self.width
            .set(if cfg.menu_fixed_width { cfg.menu_width as i32 } else { 0 });
        let items = (self.build)(if self.width.get() > 0 { Some(300) } else { None });
        let placement = match icon {
            Some(icon) => {
                // Below the top bar, which is where the work area starts.
                let display = self.anchor.display();
                let top = display
                    .monitor_at_point(icon.x(), icon.y())
                    .map(|m| m.workarea().y())
                    .unwrap_or(0);
                Placement::At(icon.x(), (icon.y() + icon.height()).max(top))
            }
            None => Placement::Pointer,
        };
        self.open(&items, placement);
    }

    /// Show `items` as a menu at the pointer: a folder picked in the top-bar
    /// menu opens like a submenu, right where it was clicked.
    pub fn show_items(self: &Rc<Self>, items: &[MenuItem]) {
        if self.state.get() != State::Closed {
            self.close();
        }
        let cfg = (self.settings)();
        self.width
            .set(if cfg.menu_fixed_width { cfg.menu_width as i32 } else { 0 });
        self.open(items, Placement::Pointer);
    }

    fn open(self: &Rc<Self>, items: &[MenuItem], placement: Placement) {
        self.keys.borrow_mut().clear();
        let menu = self.gtk_menu(items);
        let weak = Rc::downgrade(self);
        menu.connect_hide(move |_| {
            // GTK hides the menu before emitting the chosen item's "activate",
            // so focus is already heading back to the previous app when we paste.
            if let Some(p) = weak.upgrade() {
                p.finish();
            }
        });
        menu.set_attach_widget(Some(&self.anchor));
        menu.show_all();
        *self.menu.borrow_mut() = Some(menu);

        // Finding the pointer and getting focus both take a moment; do them together.
        self.located.set(true);
        match placement {
            Placement::Cursor | Placement::Pointer => {
                let caret = if matches!(placement, Placement::Cursor) {
                    shell_caret()
                } else {
                    None
                };
                if let Some(caret) = caret {
                    // Just below the line being typed on, like autocompletion.
                    self.anchor.move_(caret.x(), caret.y() + caret.height());
                } else if let Some((x, y)) = shell_pointer() {
                    self.anchor.move_(x, y);
                } else {
                    self.located.set(false);
                    self.locate_pointer(); // no helper extension: ask XWayland
                }
            }
            Placement::Centre => self.place_centred(),
            Placement::At(x, y) => self.anchor.move_(x, y),
        }
        self.state.set(State::Waiting);
        self.started.set(Instant::now());
        self.retried.set(false);
        present(&self.anchor);
        let weak = Rc::downgrade(self);
        *self.poll_id.borrow_mut() = Some(glib::timeout_add_local(POLL, move || match weak.upgrade() {
            Some(p) => p.poll(),
            None => glib::ControlFlow::Break,
        }));
    }

    fn locate_pointer(self: &Rc<Self>) {
        let display = self.probe.display();
        let mut area: Option<gdk::Rectangle> = None;
        for i in 0..display.n_monitors() {
            if let Some(monitor) = display.monitor(i) {
                let geometry = monitor.geometry();
                area = Some(match area {
                    Some(a) => a.union(&geometry),
                    None => geometry,
                });
            }
        }
        if let Some(area) = area {
            self.probe.move_(area.x(), area.y());
            self.probe.resize(area.width(), area.height());
        }
        self.probe.show();
        // A motion event brings the real position; if the pointer was already
        // where X11 last saw it, none comes, so don't wait long.
        let weak = Rc::downgrade(self);
        *self.locate_id.borrow_mut() = Some(glib::timeout_add_local_once(LOCATE, move || {
            if let Some(p) = weak.upgrade() {
                p.locate_id.borrow_mut().take();
                p.pointer_located();
            }
        }));
    }

    fn on_probe_motion(self: &Rc<Self>) {
        if self.located.get() {
            return;
        }
        if let Some(id) = self.locate_id.borrow_mut().take() {
            id.remove();
            let weak = Rc::downgrade(self);
            glib::idle_add_local_once(move || {
                if let Some(p) = weak.upgrade() {
                    p.pointer_located();
                }
            });
        }
    }

    fn pointer_located(&self) {
        if self.located.get() {
            return;
        }
        let pointer = self.probe.display().default_seat().and_then(|seat| seat.pointer());
        self.probe.hide();
        if let Some(pointer) = pointer {
            let (_screen, x, y) = pointer.position();
            // The menu hangs down-right from the pointer; GTK flips or slides it
            // back on screen near the edges.
            self.anchor.move_(x, y);
        }
        self.located.set(true);
    }

    fn place_centred(&self) {
        let display = self.anchor.display();
        let Some(monitor) = display.primary_monitor().or_else(|| display.monitor(0)) else {
            return;
        };
        let area = monitor.workarea();
        let natural = self.menu.borrow().as_ref().map(|m| m.preferred_size().1);
        let (w, h) = natural.map(|n| (n.width, n.height)).unwrap_or((0, 0));
        self.anchor.move_(
            area.x() + ((area.width() - w) / 2).max(0),
            area.y() + ((area.height() - h) / 2).max(0),
        );
    }

    fn close(&self) {
        let menu = self.menu.borrow().clone();
        match menu {
            Some(menu) if menu.is_visible() => menu.popdown(), // -> finish()
            _ => self.finish(),
        }
    }

    fn finish(&self) {
        self.state.set(State::Closed);
        if let Some(id) = self.poll_id.borrow_mut().take() {
            id.remove();
        }
        if let Some(id) = self.locate_id.borrow_mut().take() {
            id.remove();
        }
        self.located.set(true);
        self.probe.hide();
        self.anchor.hide();
    }

    /// Whether the window manager's _NET_ACTIVE_WINDOW is our anchor.
    fn anchor_is_active(&self) -> bool {
        if !self.anchor.is_visible() {
            return false;
        }
        let Some(ours) = self
            .anchor
            .window()
            .and_then(|w| w.downcast::<gdkx11::X11Window>().ok())
        else {
            return false;
        };
        let Some(root) = GtkWindowExt::screen(&self.anchor).and_then(|s| s.root_window()) else {
            return false;
        };
        let property = gdk::Atom::intern("_NET_ACTIVE_WINDOW");
        let window_type = gdk::Atom::intern("WINDOW");
        // Format-32 X properties come back as C longs.
        match gdk::property_get(&root, &property, &window_type, 0, 1, 0) {
            Some((_, 32, data)) if data.len() >= std::mem::size_of::<libc::c_ulong>() => {
                let mut bytes = [0u8; std::mem::size_of::<libc::c_ulong>()];
                let n = bytes.len();
                bytes.copy_from_slice(&data[..n]);
                libc::c_ulong::from_ne_bytes(bytes) == ours.xid()
            }
            _ => false,
        }
    }

    fn poll(self: &Rc<Self>) -> glib::ControlFlow {
        let active = self.anchor_is_active();
        match self.state.get() {
            State::Waiting => {
                let waited = self.started.get().elapsed();
                if active && self.located.get() {
                    self.state.set(State::Open);
                    self.grab_tries.set(0);
                    self.popup();
                } else if waited > FOCUS_TIMEOUT {
                    eprintln!("clipnest: GNOME didn't give the menu keyboard focus; not showing it");
                    self.poll_id.borrow_mut().take();
                    self.finish();
                    return glib::ControlFlow::Break;
                } else if !self.retried.get() && waited > FOCUS_TIMEOUT / 2 {
                    self.retried.set(true);
                    present(&self.anchor);
                }
                glib::ControlFlow::Continue
            }
            State::Open if !active => {
                self.poll_id.borrow_mut().take();
                self.close();
                glib::ControlFlow::Break
            }
            State::Open => glib::ControlFlow::Continue,
            State::Closed => {
                self.poll_id.borrow_mut().take();
                glib::ControlFlow::Break
            }
        }
    }

    fn popup(self: &Rc<Self>) {
        let Some(menu) = self.menu.borrow().clone() else { return };
        let Some(gdk_window) = self.anchor.window() else { return };
        let time = gdk_window
            .clone()
            .downcast::<gdkx11::X11Window>()
            .map(|w| gdkx11::functions::x11_get_server_time(&w))
            .unwrap_or(0);
        // A key-press trigger makes GTK open the menu in keyboard mode.
        let mut event = gdk::Event::new(gdk::EventType::KeyPress);
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = ToGlibPtrMut::to_glib_none_mut(&mut event).0;
            let key = raw as *mut gdk::ffi::GdkEventKey;
            (*key).window = gdk_window.to_glib_full();
            (*key).send_event = 1;
            (*key).time = time;
        }
        if let Some(keyboard) = self.anchor.display().default_seat().and_then(|s| s.keyboard()) {
            event.set_device(Some(&keyboard));
        }
        menu.popup_at_widget(
            &self.anchor,
            gdk::Gravity::NorthWest,
            gdk::Gravity::NorthWest,
            Some(&event),
        );
        if !menu.is_visible() {
            // The grab can fail while another grab (e.g. a held mouse button) is active.
            self.grab_tries.set(self.grab_tries.get() + 1);
            if self.grab_tries.get() < 4 {
                let weak = Rc::downgrade(self);
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    if let Some(p) = weak.upgrade() {
                        if p.state.get() == State::Open {
                            p.popup();
                        }
                    }
                });
            } else {
                eprintln!("clipnest: couldn't grab the keyboard for the menu");
                self.finish();
            }
        }
    }

    // -- building the GTK menu ------------------------------------------------

    fn gtk_menu(self: &Rc<Self>, items: &[MenuItem]) -> gtk::Menu {
        let menu = gtk::Menu::new();
        menu.set_reserve_toggle_size(false);
        if self.width.get() > 0 {
            menu.set_size_request(self.width.get(), -1);
        }
        let weak = Rc::downgrade(self);
        menu.connect_key_press_event(move |_, event| match weak.upgrade() {
            Some(p) => p.on_menu_key(event),
            None => glib::Propagation::Proceed,
        });
        // Room for "123." so numbers line up in their own column.
        let number_chars = items
            .iter()
            .filter_map(|i| i.number.as_ref())
            .map(|n| n.len() as i32 + 1)
            .max();
        let rows: Vec<&MenuItem> = items.iter().filter(|i| !i.toolbar).collect();
        for item in &rows {
            menu.append(&self.gtk_item(item, number_chars.unwrap_or(0)));
        }
        // Actions go side by side as icons in the last row (GtkMenu is a grid).
        let row = rows.len() as u32;
        for (column, item) in items.iter().filter(|i| i.toolbar).enumerate() {
            let column = column as u32;
            menu.attach(&self.toolbar_item(item), column, column + 1, row, row + 1);
        }
        menu
    }

    fn toolbar_item(self: &Rc<Self>, item: &MenuItem) -> gtk::MenuItem {
        let widget = gtk::MenuItem::new();
        let image = gtk::Image::from_icon_name(item.icon_name.as_deref(), gtk::IconSize::Menu);
        image.set_pixel_size(18);
        image.set_halign(gtk::Align::Center);
        widget.add(&image);
        widget.set_tooltip_text(Some(item.label.trim_end_matches('.')));
        let item = item.clone();
        widget.connect_activate(move |_| run(&item));
        widget
    }

    fn gtk_item(self: &Rc<Self>, item: &MenuItem, number_chars: i32) -> gtk::MenuItem {
        if item.separator {
            return gtk::SeparatorMenuItem::new().upcast();
        }
        let cfg = (self.settings)();
        let widget = gtk::MenuItem::new();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let mut extra = 0;
        if number_chars > 0 && (item.number.is_some() || item.folder) {
            // Number first, before any thumbnail; folders keep the column blank.
            let text = item.number.as_ref().map(|n| format!("{n}.")).unwrap_or_default();
            let number = gtk::Label::new(Some(&text));
            number.set_xalign(1.0);
            number.set_width_chars(number_chars);
            extra += number.preferred_width().1 + 8;
            row.pack_start(&number, false, false, 0);
        }
        if item.folder {
            row.pack_start(
                &gtk::Image::from_icon_name(Some("folder-symbolic"), gtk::IconSize::Menu),
                false,
                false,
                0,
            );
            extra += 24;
        }
        if let (Some(path), true) = (&item.image_path, cfg.menu_show_images) {
            if let Some(pixbuf) = thumbnail_box(path, cfg.menu_thumb_width as i32, cfg.menu_thumb_height as i32) {
                extra += pixbuf.width() + 8;
                row.pack_start(&gtk::Image::from_pixbuf(Some(&pixbuf)), false, false, 0);
            }
        }
        let label = gtk::Label::new(Some(&item.text));
        label.set_xalign(0.0);
        if self.width.get() > 0 {
            // Tiny natural width + explicit width = exactly fills the row.
            label.set_ellipsize(pango::EllipsizeMode::End);
            label.set_max_width_chars(1);
            label.set_size_request((self.width.get() - ROW_CHROME - extra).max(40), -1);
        }
        row.pack_start(&label, true, true, 0);
        if let Some(key) = item.key {
            self.keys.borrow_mut().insert(key, widget.clone());
        }
        widget.add(&row);
        widget.set_sensitive(item.enabled);
        if let Some(tooltip) = &item.tooltip {
            widget.set_tooltip_text(Some(tooltip));
        }
        if !item.children.is_empty() {
            widget.set_submenu(Some(&self.gtk_menu(&item.children)));
        } else if item.callback.is_some() {
            let item = item.clone();
            widget.connect_activate(move |_| run(&item));
        }
        widget
    }

    /// 1…9 / 0 pick items 1…10, wherever they sit in the menu.
    /// With Shift held, the item is pasted as plain text.
    fn on_menu_key(&self, event: &gdk::EventKey) -> glib::Propagation {
        let mods = event.state() & gtk::accelerator_get_default_mod_mask();
        if !(mods - gdk::ModifierType::SHIFT_MASK).is_empty() {
            return glib::Propagation::Proceed;
        }
        // Shift+1 is "!" on most layouts, so look at the unshifted key.
        let keymap = gdk::Keymap::for_display(&self.anchor.display());
        let keyval = keymap
            .and_then(|k| {
                k.translate_keyboard_state(
                    event.hardware_keycode() as u32,
                    gdk::ModifierType::empty(),
                    event.group() as i32,
                )
            })
            .map(|(keyval, ..)| keyval)
            .unwrap_or_else(|| *event.keyval());
        let Some(digit) = gdk::keys::Key::from(keyval).to_unicode().filter(char::is_ascii_digit) else {
            return glib::Propagation::Proceed;
        };
        let Some(widget) = self.keys.borrow().get(&digit).cloned() else {
            return glib::Propagation::Proceed;
        };
        // Close first, as a click does, so focus is heading back before pasting.
        if let Some(menu) = self.menu.borrow().clone() {
            menu.deactivate();
        }
        widget.activate();
        glib::Propagation::Stop
    }
}

fn run(item: &MenuItem) {
    let shift = gtk::current_event_state()
        .map(|state| state.contains(gdk::ModifierType::SHIFT_MASK))
        .unwrap_or(false);
    match (&item.plain_callback, &item.callback) {
        (Some(plain), _) if shift => plain(),
        (_, Some(callback)) => callback(),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn entry(id: i64, kind: &str, content: &str, summary: &str) -> HistoryItem {
        HistoryItem {
            id,
            kind: kind.into(),
            content: content.into(),
            digest: format!("d{id}"),
            summary: summary.into(),
            created: 0.0,
            pinned: false,
        }
    }

    fn labels(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|i| if i.separator { "---".into() } else { i.label.clone() })
            .collect()
    }

    #[test]
    fn titles_are_cut_to_length() {
        assert_eq!(
            item_title("jaise other kits use karte hain", 20),
            "jaise other kits ..."
        );
        assert_eq!(item_title("DB_CONNECTION=mysql", 20), "DB_CONNECTION=mysql");
        assert_eq!(item_title("  first line\nsecond line", 20), "first line");
        assert_eq!(item_title(" \n ", 20), "(whitespace)");
    }

    #[test]
    fn layout_numbers_items_and_folds_older_ones() {
        let used = Rc::new(RefCell::new(Vec::new()));
        let log = used.clone();
        let nothing: Callback = Rc::new(|| {});
        let actions = MenuActions {
            use_item: Rc::new(move |id, panel, plain| log.borrow_mut().push((id, panel, plain))),
            search: nothing.clone(),
            clear: nothing.clone(),
            preferences: nothing.clone(),
            quit: nothing,
            open_folder: Rc::new(|_| {}),
        };
        let mut entries = vec![entry(100, "image", "x.png", "Image 1×1")];
        entries.extend((1..=18).map(|n| entry(n, "text", &format!("item {n}"), "")));
        let items = build_menu(
            &Settings::default(),
            &entries,
            &|e| PathBuf::from(&e.content),
            &actions,
            false,
            None,
            None,
        );
        let l = labels(&items);
        assert_eq!(l[0], "History");
        assert_eq!(l[1], "1. (Image)");
        assert_eq!(l[2], "2. item 1");
        assert_eq!(l[15], "15. item 14");
        assert_eq!(l[16], "16 - 19");
        assert!(items[16].folder);
        assert_eq!(items[16].children[0].label, "16. item 15");
        assert_eq!(
            &l[17..],
            [
                "---",
                "Search History...",
                "Clear History",
                "Preferences...",
                "Quit Clipnest"
            ]
        );
        assert!(items[18..].iter().all(|i| i.toolbar && i.icon_name.is_some()));
        // the GTK menu shows the number in its own column, before any thumbnail
        assert_eq!(
            (items[1].number.as_deref(), items[1].text.as_str()),
            (Some("1"), "(Image)")
        );
        assert!(items[1].image_path.is_some());
        // 1…9 select with their digit, the 10th with 0, later ones have none
        assert_eq!(items[1].key, Some('1'));
        assert_eq!(items[10].key, Some('0'));
        assert_eq!(items[11].key, None);

        (items[2].callback.as_ref().unwrap())();
        (items[16].children[0].plain_callback.as_ref().unwrap())();
        assert_eq!(*used.borrow(), [(1, false, false), (15, false, true)]);
    }

    #[test]
    fn title_length_override_and_empty_history() {
        let nothing: Callback = Rc::new(|| {});
        let actions = MenuActions {
            use_item: Rc::new(|_, _, _| {}),
            search: nothing.clone(),
            clear: nothing.clone(),
            preferences: nothing.clone(),
            quit: nothing,
            open_folder: Rc::new(|_| {}),
        };
        let entries = [entry(1, "text", "a fairly long clipboard entry that keeps going", "")];
        let path = |e: &HistoryItem| PathBuf::from(&e.content);
        let cfg = Settings::default();
        assert_eq!(
            build_menu(&cfg, &entries, &path, &actions, false, None, None)[1].label,
            "1. a fairly long cli..."
        );
        assert_eq!(
            build_menu(&cfg, &entries, &path, &actions, false, Some(300), None)[1].label,
            "1. a fairly long clipboard entry that keeps going"
        );
        assert_eq!(
            labels(&build_menu(&cfg, &[], &path, &actions, false, None, None))[..2],
            ["History", "(nothing copied yet)"]
        );
    }

    #[test]
    fn top_bar_folders_open_as_their_own_menu() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let log = opened.clone();
        let nothing: Callback = Rc::new(|| {});
        let actions = MenuActions {
            use_item: Rc::new(|_, _, _| {}),
            search: nothing.clone(),
            clear: nothing.clone(),
            preferences: nothing.clone(),
            quit: nothing,
            open_folder: Rc::new(move |items: Vec<MenuItem>| {
                log.borrow_mut().extend(items.into_iter().map(|i| i.label))
            }),
        };
        let entries: Vec<HistoryItem> = (1..=17).map(|n| entry(n, "text", &format!("item {n}"), "")).collect();
        let items = build_menu(
            &Settings::default(),
            &entries,
            &|e| PathBuf::from(&e.content),
            &actions,
            true,
            None,
            None,
        );
        let folder = &items[16];
        assert_eq!(folder.label, "16 - 17");
        assert!(folder.children.is_empty() && !folder.folder);
        assert_eq!(folder.icon_name.as_deref(), Some("go-next-symbolic"));
        (folder.callback.as_ref().unwrap())();
        assert_eq!(*opened.borrow(), ["16. item 16", "17. item 17"]);
    }

    #[test]
    fn menus_shrink_to_fit_the_screen() {
        // 15 entries, 15 inline, folders of 10, room for 12 rows: 11 inline + "12 - 15"
        assert_eq!(fit_layout(15, 15, 10, 12), (11, 10));
        assert_eq!(fit_layout(15, 15, 10, 20), (15, 10)); // fits already
        assert_eq!(fit_layout(50, 15, 10, 8), (3, 10)); // 3 + folders 4-13 … 44-50 = 8 rows
                                                        // folders of 1 save no rows: list 10, put the other 6 in one folder
        assert_eq!(fit_layout(16, 22, 1, 11), (10, 6));
        assert_eq!(fit_layout(50, 15, 10, 2), (1, 49));
        let nothing: Callback = Rc::new(|| {});
        let actions = MenuActions {
            use_item: Rc::new(|_, _, _| {}),
            search: nothing.clone(),
            clear: nothing.clone(),
            preferences: nothing.clone(),
            quit: nothing,
            open_folder: Rc::new(|_| {}),
        };
        let entries: Vec<HistoryItem> = (1..=15).map(|n| entry(n, "text", &format!("item {n}"), "")).collect();
        let items = build_menu(
            &Settings::default(),
            &entries,
            &|e| PathBuf::from(&e.content),
            &actions,
            true,
            None,
            Some(12),
        );
        let l = labels(&items);
        assert_eq!(l[11], "11. item 11");
        assert_eq!(l[12], "12 - 15");
        assert_eq!(l[13], "---");
    }
}
