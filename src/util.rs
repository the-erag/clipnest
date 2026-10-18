//! Small helpers shared by the menus, popup and preferences.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use gdk_pixbuf::Pixbuf;
use gtk::prelude::*;

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// One-line, whitespace-collapsed, truncated title for a clip.
pub fn summarize(text: &str, length: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        return "(whitespace)".into();
    }
    if line.chars().count() > length {
        let cut: String = line.chars().take(length.saturating_sub(1).max(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        line
    }
}

pub fn relative_time(timestamp: f64, now: f64) -> String {
    let seconds = (now - timestamp).max(0.0) as i64;
    match seconds {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s if s < 7 * 86400 => format!("{} d ago", s / 86400),
        _ => glib::DateTime::from_unix_local(timestamp as i64)
            .and_then(|d| d.format("%d %b %Y"))
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

/// 1234567 -> "1,234,567"
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn describe_text(text: &str) -> String {
    let lines = text.matches('\n').count() + 1;
    let chars = format!("{} chars", thousands(text.chars().count()));
    if lines > 1 {
        format!("{} lines · {}", thousands(lines), chars)
    } else {
        chars
    }
}

thread_local! {
    static THUMBNAILS: RefCell<HashMap<(String, i32, i32), Option<Pixbuf>>> = RefCell::new(HashMap::new());
}

/// Pixbuf scaled to fit a width×height box (cached), or None if it can't be loaded.
pub fn thumbnail_box(path: &Path, width: i32, height: i32) -> Option<Pixbuf> {
    let key = (path.to_string_lossy().into_owned(), width, height);
    THUMBNAILS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 256 {
            cache.clear();
        }
        cache
            .entry(key)
            .or_insert_with(|| Pixbuf::from_file_at_scale(path, width, height, true).ok())
            .clone()
    })
}

pub fn thumbnail(path: &Path, size: i32) -> Option<Pixbuf> {
    thumbnail_box(path, size, size)
}

pub fn thumbnail_png(path: &Path, size: i32) -> Option<Vec<u8>> {
    thumbnail(path, size)?.save_to_bufferv("png", &[]).ok()
}

/// Show and focus a window.
///
/// A fresh X server timestamp keeps GNOME's focus-stealing prevention from
/// leaving the window unfocused behind whatever had focus before.
pub fn present(window: &impl IsA<gtk::Window>) {
    let window = window.upcast_ref::<gtk::Window>();
    window.show();
    if let Some(x11) = window.window().and_then(|w| w.downcast::<gdkx11::X11Window>().ok()) {
        window.present_with_time(gdkx11::functions::x11_get_server_time(&x11));
    } else {
        window.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_collapses_and_truncates() {
        assert_eq!(summarize("  hello\n\n  world  ", 40), "hello world");
        assert_eq!(summarize("abcdefghij", 5), "abcd…");
        assert_eq!(summarize(" \n\t", 10), "(whitespace)");
    }

    #[test]
    fn relative_times() {
        let now = 1_000_000.0;
        assert_eq!(relative_time(now - 5.0, now), "just now");
        assert_eq!(relative_time(now - 120.0, now), "2 min ago");
        assert_eq!(relative_time(now - 7200.0, now), "2 h ago");
        assert_eq!(relative_time(now - 3.0 * 86400.0, now), "3 d ago");
    }

    #[test]
    fn number_formatting() {
        assert_eq!(thousands(7), "7");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(describe_text("a\nb"), "2 lines · 3 chars");
    }
}
