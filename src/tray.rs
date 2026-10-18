//! Top-bar icon with the history menu.
//!
//! Where the desktop still hosts legacy (XEmbed) tray icons, as GNOME's
//! AppIndicator extension does, Clipnest shows one of those: clicks then reach
//! Clipnest, which opens its own menu under the icon (left or right click) or
//! the search popup (middle click).
//!
//! Otherwise it implements the StatusNotifierItem and com.canonical.dbusmenu
//! D-Bus protocols directly, so the icon works with the AppIndicator extension
//! and KDE without needing libayatana-appindicator; the desktop then draws the
//! menu. Clicks: left = menu, double-click or middle-click = search popup.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use glib::{ToVariant, Variant};

use crate::menu::{Callback, MenuItem};
use crate::paths;
use crate::status_icon::StatusIcon;
use crate::util::thumbnail_png;
use crate::APP_NAME;

const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

const ITEM_XML: &str = r#"
<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
    <method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
    <method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
    <method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
    <signal name="NewIcon"/>
    <signal name="NewTitle"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus"><arg type="s"/></signal>
  </interface>
</node>"#;

const MENU_XML: &str = r#"
<node>
  <interface name="com.canonical.dbusmenu">
    <property name="Version" type="u" access="read"/>
    <property name="TextDirection" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="IconThemePath" type="as" access="read"/>
    <method name="GetLayout">
      <arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
      <arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/>
    </method>
    <method name="GetGroupProperties">
      <arg type="ai" direction="in"/><arg type="as" direction="in"/>
      <arg type="a(ia{sv})" direction="out"/>
    </method>
    <method name="GetProperty">
      <arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/>
    </method>
    <method name="Event">
      <arg type="i" direction="in"/><arg type="s" direction="in"/>
      <arg type="v" direction="in"/><arg type="u" direction="in"/>
    </method>
    <method name="EventGroup">
      <arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg type="i" direction="in"/><arg type="b" direction="out"/>
    </method>
    <method name="AboutToShowGroup">
      <arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/>
    </method>
    <signal name="ItemsPropertiesUpdated">
      <arg type="a(ia{sv})"/><arg type="a(ias)"/>
    </signal>
    <signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
    <signal name="ItemActivationRequested"><arg type="i"/><arg type="u"/></signal>
  </interface>
</node>"#;

type Props = HashMap<String, Variant>;

/// One menu entry as dbusmenu sees it.
struct Node {
    props: Props,
    children: Vec<i32>,
    callback: Option<Callback>,
}

fn properties(item: &MenuItem, show_images: bool) -> Props {
    let mut props = Props::new();
    if item.separator {
        props.insert("type".into(), "separator".to_variant());
        return props;
    }
    if !item.label.is_empty() {
        // dbusmenu treats "_" as a mnemonic marker
        props.insert("label".into(), item.label.replace('_', "__").to_variant());
    }
    if !item.enabled {
        props.insert("enabled".into(), false.to_variant());
    }
    if !item.children.is_empty() {
        props.insert("children-display".into(), "submenu".to_variant());
    }
    if let Some(icon) = &item.icon_name {
        props.insert("icon-name".into(), icon.to_variant());
    }
    if let (Some(path), true) = (&item.image_path, show_images) {
        if let Some(png) = thumbnail_png(path, 32) {
            props.insert("icon-data".into(), png.to_variant());
        }
    }
    props
}

/// The menu tree with dbusmenu ids.
///
/// Every rebuild gets ids never used before. Menu hosts (GNOME's AppIndicator
/// extension included) cache items by id and only re-read a known id's type on
/// LayoutUpdated, so reusing ids leaves stale labels and separators behind.
pub struct MenuTree {
    nodes: HashMap<i32, Node>,
    next_id: i32,
    pub revision: u32,
}

impl MenuTree {
    pub fn new() -> MenuTree {
        let mut tree = MenuTree {
            nodes: HashMap::new(),
            next_id: 1,
            revision: 0,
        };
        tree.set_items(&[], true);
        tree
    }

    fn add(&mut self, item: &MenuItem, show_images: bool) -> i32 {
        let id = self.next_id;
        self.next_id = if self.next_id < i32::MAX { self.next_id + 1 } else { 1 };
        let children = item.children.iter().map(|c| self.add(c, show_images)).collect();
        self.nodes.insert(
            id,
            Node {
                props: properties(item, show_images),
                children,
                callback: item.callback.clone(),
            },
        );
        id
    }

    pub fn set_items(&mut self, items: &[MenuItem], show_images: bool) {
        self.nodes.clear();
        let children = items.iter().map(|i| self.add(i, show_images)).collect();
        let root_props = Props::from([("children-display".to_string(), "submenu".to_variant())]);
        self.nodes.insert(
            0,
            Node {
                props: root_props,
                children,
                callback: None,
            },
        );
        self.revision += 1;
    }

    fn props(&self, id: i32, names: &[String]) -> Props {
        let props = self.nodes.get(&id).map(|n| n.props.clone()).unwrap_or_default();
        if names.is_empty() {
            props
        } else {
            props.into_iter().filter(|(k, _)| names.contains(k)).collect()
        }
    }

    /// (ia{sv}av) for an item and `depth` levels of children (-1: all).
    pub fn layout(&self, id: i32, depth: i32, names: &[String]) -> Variant {
        let children: Vec<Variant> = match (depth, self.nodes.get(&id)) {
            (0, _) | (_, None) => Vec::new(),
            (_, Some(node)) => node
                .children
                .iter()
                .map(|c| self.layout(*c, depth - 1, names))
                .collect(),
        };
        (id, self.props(id, names), children).to_variant()
    }

    pub fn contains(&self, id: i32) -> bool {
        self.nodes.contains_key(&id)
    }

    pub fn callback(&self, id: i32) -> Option<Callback> {
        self.nodes.get(&id).and_then(|n| n.callback.clone())
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<i32> {
        self.nodes.keys().copied().collect()
    }
}

pub struct Tray {
    connection: gio::DBusConnection,
    tree: RefCell<MenuTree>,
    build: Box<dyn Fn() -> (Vec<MenuItem>, bool)>,
    on_activate: Callback,
    pending: Cell<bool>,
    status_icon: RefCell<Option<StatusIcon>>,
}

/// How long to wait for a legacy tray to take the icon before falling back to
/// an AppIndicator.
const EMBED_WAIT: Duration = Duration::from_millis(1500);

thread_local! {
    // gio wants D-Bus handlers to be Send + Sync, so they can't hold the Rc;
    // GDBus calls them on the main thread, where they find the tray here.
    static TRAY: RefCell<std::rc::Weak<Tray>> = const { RefCell::new(std::rc::Weak::new()) };
}

fn current() -> Option<Rc<Tray>> {
    TRAY.with(|t| t.borrow().upgrade())
}

fn invoke_later(callback: Callback) {
    glib::idle_add_local_once(move || callback());
}

impl Tray {
    /// `build` returns the menu items and whether to show image thumbnails;
    /// `open_menu` shows Clipnest's own menu below the given icon area.
    pub fn new(
        connection: gio::DBusConnection,
        build: impl Fn() -> (Vec<MenuItem>, bool) + 'static,
        on_activate: Callback,
        open_menu: Rc<dyn Fn(Option<gdk::Rectangle>)>,
    ) -> Rc<Tray> {
        let tray = Rc::new(Tray {
            connection: connection.clone(),
            tree: RefCell::new(MenuTree::new()),
            build: Box::new(build),
            on_activate,
            pending: Cell::new(false),
            status_icon: RefCell::new(None),
        });
        TRAY.with(|t| *t.borrow_mut() = Rc::downgrade(&tray));

        let menu_info = gio::DBusNodeInfo::for_xml(MENU_XML)
            .ok()
            .and_then(|n| n.lookup_interface(MENU_IFACE));
        if let Some(info) = menu_info {
            let result = connection.register_object(
                MENU_PATH,
                &info,
                |_, _, _, _, method, params, invocation| {
                    if let Some(tray) = current() {
                        tray.menu_call(method, &params, invocation);
                    }
                },
                |_, _, _, _, property| match property {
                    "Version" => 3u32.to_variant(),
                    "TextDirection" => "ltr".to_variant(),
                    "Status" => "normal".to_variant(),
                    _ => Vec::<String>::new().to_variant(), // IconThemePath
                },
                |_, _, _, _, _, _| false,
            );
            if let Err(err) = result {
                eprintln!("clipnest: tray menu unavailable: {err}");
            }
        }

        let item_info = gio::DBusNodeInfo::for_xml(ITEM_XML)
            .ok()
            .and_then(|n| n.lookup_interface(ITEM_IFACE));
        if let Some(info) = item_info {
            let result = connection.register_object(
                ITEM_PATH,
                &info,
                |_, _, _, _, method, _params, invocation| {
                    if method == "Activate" || method == "SecondaryActivate" {
                        if let Some(tray) = current() {
                            invoke_later(tray.on_activate.clone());
                        }
                    }
                    invocation.return_value(None);
                },
                |_, _, _, _, property| item_property(property),
                |_, _, _, _, _, _| false,
            );
            if let Err(err) = result {
                eprintln!("clipnest: tray icon unavailable: {err}");
            }
        }

        if std::env::var("CLIPNEST_TRAY").as_deref() != Ok("indicator") {
            let icon = StatusIcon::new("clipnest-panel", APP_NAME);
            let weak = Rc::downgrade(&tray);
            icon.connect_click(move |button| {
                let Some(tray) = weak.upgrade() else { return };
                if button == 2 {
                    invoke_later(tray.on_activate.clone());
                } else {
                    let area = tray.status_icon.borrow().as_ref().and_then(StatusIcon::geometry);
                    let open = open_menu.clone();
                    glib::idle_add_local_once(move || open(area));
                }
            });
            *tray.status_icon.borrow_mut() = Some(icon);
            let weak = Rc::downgrade(&tray);
            glib::timeout_add_local_once(EMBED_WAIT, move || {
                if let Some(tray) = weak.upgrade() {
                    let embedded = tray.status_icon.borrow().as_ref().is_some_and(StatusIcon::is_embedded);
                    if !embedded {
                        if let Some(icon) = tray.status_icon.borrow_mut().take() {
                            icon.set_visible(false);
                        }
                        tray.watch_for_host();
                    }
                }
            });
        } else {
            tray.watch_for_host();
        }
        tray.rebuild();
        tray
    }

    /// Show the AppIndicator, (re-)registering it whenever a tray host
    /// appears, e.g. after gnome-shell restarts.
    fn watch_for_host(&self) {
        // Watches for the whole run; dropping the id doesn't stop it.
        let _ = gio::bus_watch_name_on_connection(
            &self.connection,
            WATCHER,
            gio::BusNameWatcherFlags::NONE,
            |connection, _, _| {
                connection.call(
                    Some(WATCHER),
                    "/StatusNotifierWatcher",
                    WATCHER,
                    "RegisterStatusNotifierItem",
                    Some(&(ITEM_PATH,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    -1,
                    None::<&gio::Cancellable>,
                    |result| {
                        if let Err(err) = result {
                            eprintln!("clipnest: could not register tray icon: {err}");
                        }
                    },
                );
            },
            |_, _| {},
        );
    }

    pub fn schedule_rebuild(self: &Rc<Self>) {
        if self.pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            if let Some(tray) = weak.upgrade() {
                tray.pending.set(false);
                tray.rebuild();
            }
        });
    }

    pub fn rebuild(&self) {
        let (items, show_images) = (self.build)();
        let revision = {
            let mut tree = self.tree.borrow_mut();
            tree.set_items(&items, show_images);
            tree.revision
        };
        let _ = self.connection.emit_signal(
            None,
            MENU_PATH,
            MENU_IFACE,
            "LayoutUpdated",
            Some(&(revision, 0i32).to_variant()),
        );
    }

    fn menu_call(&self, method: &str, params: &Variant, invocation: gio::DBusMethodInvocation) {
        let tree = self.tree.borrow();
        match method {
            "GetLayout" => {
                let Some((parent, depth, names)) = params.get::<(i32, i32, Vec<String>)>() else {
                    return invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", "bad arguments");
                };
                if !tree.contains(parent) {
                    return invocation.return_dbus_error(&format!("{MENU_IFACE}.Error"), "no such item");
                }
                let layout = tree.layout(parent, depth, &names);
                invocation.return_value(Some(&Variant::tuple_from_iter([tree.revision.to_variant(), layout])));
            }
            "GetGroupProperties" => {
                let (ids, names) = params.get::<(Vec<i32>, Vec<String>)>().unwrap_or_default();
                let result: Vec<(i32, Props)> = ids
                    .into_iter()
                    .filter(|id| tree.contains(*id))
                    .map(|id| (id, tree.props(id, &names)))
                    .collect();
                invocation.return_value(Some(&(result,).to_variant()));
            }
            "GetProperty" => {
                let (id, name) = params.get::<(i32, String)>().unwrap_or_default();
                match tree.props(id, &[]).get(&name) {
                    Some(value) => {
                        invocation.return_value(Some(&Variant::tuple_from_iter([Variant::from_variant(value)])))
                    }
                    None => invocation.return_dbus_error(&format!("{MENU_IFACE}.Error"), "no such property"),
                }
            }
            "Event" => {
                if let Some((id, event, _, _)) = params.get::<(i32, String, Variant, u32)>() {
                    if event == "clicked" {
                        if let Some(callback) = tree.callback(id) {
                            invoke_later(callback);
                        }
                    }
                }
                invocation.return_value(None);
            }
            "EventGroup" => {
                let (events,) = params.get::<(Vec<(i32, String, Variant, u32)>,)>().unwrap_or_default();
                let mut missing = Vec::new();
                for (id, event, _, _) in events {
                    match tree.callback(id) {
                        Some(callback) if event == "clicked" => invoke_later(callback),
                        _ if !tree.contains(id) => missing.push(id),
                        _ => {}
                    }
                }
                invocation.return_value(Some(&(missing,).to_variant()));
            }
            "AboutToShow" => invocation.return_value(Some(&(false,).to_variant())),
            "AboutToShowGroup" => invocation.return_value(Some(&(Vec::<i32>::new(), Vec::<i32>::new()).to_variant())),
            _ => invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method),
        }
    }
}

fn item_property(name: &str) -> Variant {
    match name {
        "Category" => "ApplicationStatus".to_variant(),
        "Id" => "clipnest".to_variant(),
        "Title" => APP_NAME.to_variant(),
        "Status" => "Active".to_variant(),
        "WindowId" => 0i32.to_variant(),
        "IconName" => "clipnest-symbolic".to_variant(),
        "IconThemePath" => paths::icon_dir().to_string_lossy().to_variant(),
        "ToolTip" => ("", Vec::<(i32, i32, Vec<u8>)>::new(), APP_NAME, "Clipboard history").to_variant(),
        "ItemIsMenu" => false.to_variant(),
        "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH.to_string())
            .unwrap()
            .to_variant(),
        _ => "".to_variant(), // AttentionIconName, OverlayIconName
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_never_reuse_ids() {
        let mut tree = MenuTree::new();
        tree.set_items(
            &[
                MenuItem::new("A"),
                MenuItem::separator(),
                MenuItem {
                    children: vec![MenuItem::new("B1")],
                    ..MenuItem::new("B")
                },
            ],
            true,
        );
        let first: Vec<i32> = tree.ids().into_iter().filter(|i| *i != 0).collect();
        tree.set_items(
            &[MenuItem::new("Open"), MenuItem::separator(), MenuItem::new("Quit")],
            true,
        );
        let second: Vec<i32> = tree.ids().into_iter().filter(|i| *i != 0).collect();
        assert!(first.iter().all(|id| !second.contains(id)));
    }

    #[test]
    fn layout_variant() {
        let mut tree = MenuTree::new();
        tree.set_items(
            &[
                MenuItem {
                    enabled: false,
                    ..MenuItem::new("History")
                },
                MenuItem::separator(),
                MenuItem::new("my_file"),
                MenuItem {
                    children: vec![MenuItem::new("child")],
                    ..MenuItem::new("Folder")
                },
            ],
            true,
        );
        let layout = tree.layout(0, -1, &[]);
        assert_eq!(layout.type_().as_str(), "(ia{sv}av)");
        let (id, props, children) = layout.get::<(i32, Props, Vec<Variant>)>().unwrap();
        assert_eq!(id, 0);
        assert_eq!(props["children-display"].get::<String>().unwrap(), "submenu");
        // glib unboxes the "v" elements of an av when reading it back
        let child = |i: usize| children[i].get::<(i32, Props, Vec<Variant>)>().unwrap();
        assert_eq!(child(0).1["enabled"].get::<bool>(), Some(false));
        assert_eq!(child(1).1["type"].get::<String>().unwrap(), "separator");
        assert_eq!(child(2).1["label"].get::<String>().unwrap(), "my__file"); // "_" escaped
        assert_eq!(child(3).2.len(), 1);
    }
}
