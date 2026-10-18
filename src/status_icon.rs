//! Legacy (XEmbed) tray icon through GTK's GtkStatusIcon, which gtk-rs no
//! longer wraps because GTK deprecated it.
//!
//! GNOME's AppIndicator extension shows these icons too, but unlike an
//! AppIndicator it hands the click to the app instead of drawing its own menu,
//! so Clipnest can open its real menu (with submenus that open on hover).

use glib::prelude::*;
use glib::translate::{from_glib_full, IntoGlib, ToGlibPtr};

pub struct StatusIcon {
    object: glib::Object,
    raw: *mut gtk::ffi::GtkStatusIcon,
}

impl StatusIcon {
    pub fn new(icon_name: &str, title: &str) -> StatusIcon {
        unsafe {
            let raw = gtk::ffi::gtk_status_icon_new_from_icon_name(icon_name.to_glib_none().0);
            gtk::ffi::gtk_status_icon_set_name(raw, "clipnest".to_glib_none().0);
            gtk::ffi::gtk_status_icon_set_title(raw, title.to_glib_none().0);
            gtk::ffi::gtk_status_icon_set_tooltip_text(raw, title.to_glib_none().0);
            let object: glib::Object = from_glib_full(raw as *mut glib::gobject_ffi::GObject);
            StatusIcon { object, raw }
        }
    }

    /// Whether a tray picked the icon up.
    pub fn is_embedded(&self) -> bool {
        unsafe { gtk::ffi::gtk_status_icon_is_embedded(self.raw) != 0 }
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe { gtk::ffi::gtk_status_icon_set_visible(self.raw, visible.into_glib()) }
    }

    /// Where the icon sits on screen, when the tray says.
    pub fn geometry(&self) -> Option<gdk::Rectangle> {
        let mut area = gdk::ffi::GdkRectangle {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        let known = unsafe {
            gtk::ffi::gtk_status_icon_get_geometry(self.raw, std::ptr::null_mut(), &mut area, std::ptr::null_mut())
        };
        (known != 0 && area.width > 0).then(|| gdk::Rectangle::new(area.x, area.y, area.width, area.height))
    }

    /// Calls `f(button)` when the icon is clicked (1 left, 2 middle, 3 right).
    pub fn connect_click(&self, f: impl Fn(u32) + 'static) {
        self.object.connect_local("button-press-event", false, move |values| {
            let event = values.get(1).and_then(|v| v.get::<gdk::Event>().ok());
            if std::env::var_os("CLIPNEST_DEBUG").is_some() {
                eprintln!(
                    "clipnest: tray icon press: {:?}",
                    event.as_ref().map(|e| (e.event_type(), e.button()))
                );
            }
            // A double click also sends a 2BUTTON_PRESS; only single presses count.
            let button = event
                .filter(|e| e.event_type() == gdk::EventType::ButtonPress)
                .and_then(|e| e.button());
            if let Some(button) = button {
                f(button);
            }
            Some(true.to_value())
        });
    }
}
