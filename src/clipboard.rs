//! Watches the CLIPBOARD selection, records new entries and puts entries back.
//!
//! Clipnest runs on the X11 backend (XWayland under a Wayland session): GNOME
//! mirrors the Wayland clipboard into XWayland, and X11 clients may observe
//! ownership changes without having focus, which Wayland clients may not.
//!
//! Like Clipy, every enabled format of a copy is saved: the plain text (or
//! image, or file list) is what menus show, and HTML / RTF / PDF / file-list
//! versions are stored alongside it. Pasting from history offers all of them
//! again, so a formatted paragraph pastes formatted and copied files paste as
//! files.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gdk_pixbuf::Pixbuf;
use gtk::prelude::*;
use regex::Regex;

use crate::config::Settings;
use crate::storage::{self as st, Formats, HistoryItem, Storage};

/// Password managers (KeePassXC, KDE apps, ...) tag secrets with this target.
const SECRET_TARGETS: &[&str] = &["x-kde-passwordManagerHint"];
const FILE_TARGETS: &[&str] = &["x-special/gnome-copied-files", "text/uri-list"];
const HTML_TARGETS: &[&str] = &["text/html"];
const RTF_TARGETS: &[&str] = &["text/rtf", "application/rtf", "text/richtext"];
const PDF_TARGETS: &[&str] = &["application/pdf"];
const RICH_NAMES: &[(&str, &str)] = &[
    ("application/pdf", "PDF document"),
    ("text/rtf", "Rich text"),
    ("application/rtf", "Rich text"),
    ("text/richtext", "Rich text"),
    ("text/html", "HTML"),
];
const TEXT_TARGETS: &[&str] = &[
    "UTF8_STRING",
    "text/plain;charset=utf-8",
    "text/plain",
    "STRING",
    "TEXT",
    "COMPOUND_TEXT",
];
const IMAGE_TARGETS: &[&str] = &["image/png", "image/jpeg", "image/bmp", "image/tiff"];
const MAX_IMAGE_BYTES: usize = 25 * 1024 * 1024;
const MAX_FORMAT_BYTES: usize = 10 * 1024 * 1024;
const INFO_TEXT: u32 = 1;
const INFO_IMAGE: u32 = 2;
const INFO_RAW: u32 = 3;

/// Set CLIPNEST_DEBUG=1 to trace clipboard reads on stderr.
fn debug(message: impl FnOnce() -> String) {
    thread_local! {
        static ON: bool = std::env::var_os("CLIPNEST_DEBUG").is_some();
    }
    if ON.with(|on| *on) {
        eprintln!("clipnest: {}", message());
    }
}

fn is_url(text: &str) -> bool {
    thread_local! {
        static URL: Regex = Regex::new(r"(?i)^(https?|ftp|file|mailto):\S+$").unwrap();
    }
    URL.with(|re| re.is_match(text.trim()))
}

/// Local paths from x-special/gnome-copied-files or text/uri-list data, else [].
pub fn parse_file_list(raw: &Formats) -> Vec<String> {
    let data = match raw.get(FILE_TARGETS[0]).or_else(|| raw.get(FILE_TARGETS[1])) {
        Some(data) => String::from_utf8_lossy(data).into_owned(),
        None => return Vec::new(),
    };
    let mut lines: Vec<&str> = data.lines().collect();
    if matches!(lines.first().map(|l| l.trim()), Some("copy") | Some("cut")) {
        lines.remove(0);
    }
    let uris: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    // Browsers put web links in text/uri-list too; only local files count.
    if uris.is_empty() || !uris.iter().all(|u| u.starts_with("file://")) {
        return Vec::new();
    }
    uris.iter()
        .map(|u| {
            gio::File::for_uri(u)
                .path()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| u.to_string())
        })
        .collect()
}

/// Remembers what Clipnest itself put on the clipboard, so the echo is ignored.
#[derive(Default)]
pub struct EchoGuard(HashSet<String>);

impl EchoGuard {
    pub fn set(&mut self, digests: HashSet<String>) {
        self.0 = digests;
    }

    pub fn is_own_echo(&mut self, digest: &str) -> bool {
        if self.0.contains(digest) {
            return true;
        }
        self.0.clear();
        false
    }
}

#[derive(Default)]
pub struct PatternCache {
    source: Vec<String>,
    compiled: Vec<Regex>,
}

impl PatternCache {
    pub fn matches(&mut self, patterns: &[String], text: &str) -> bool {
        if patterns != self.source.as_slice() {
            self.source = patterns.to_vec();
            // Invalid patterns are skipped; Preferences points them out.
            self.compiled = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
        }
        self.compiled.iter().any(|re| re.is_match(text))
    }
}

/// Everything read for one clipboard change.
#[derive(Default)]
pub struct CaptureData {
    pub text: Option<String>,
    pub image: Option<Pixbuf>,
    pub raw: Formats,
}

fn describe(rich: &Formats) -> String {
    RICH_NAMES
        .iter()
        .find(|(mime, _)| rich.contains_key(*mime))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| "Data".into())
}

/// Save one capture as a history entry. Returns whether history changed.
pub fn store_capture(
    cfg: &Settings,
    storage: &Storage,
    echo: &mut EchoGuard,
    patterns: &mut PatternCache,
    data: CaptureData,
) -> bool {
    let rich: Formats = data
        .raw
        .iter()
        .filter(|(m, _)| !FILE_TARGETS.contains(&m.as_str()))
        .map(|(m, d)| (m.clone(), d.clone()))
        .collect();
    let paths = if cfg.store_files {
        parse_file_list(&data.raw)
    } else {
        Vec::new()
    };

    let is_new = if !paths.is_empty() {
        let mut formats: Formats = data
            .raw
            .iter()
            .filter(|(m, _)| FILE_TARGETS.contains(&m.as_str()))
            .map(|(m, d)| (m.clone(), d.clone()))
            .collect();
        if let Some(copied) = formats.get_mut(FILE_TARGETS[0]) {
            if copied.starts_with(b"cut") {
                // Pasting from history must never move the original files.
                *copied = [b"copy".as_slice(), &copied[3..]].concat();
            }
        }
        if echo.is_own_echo(&st::files_digest(&paths)) {
            return false;
        }
        storage.add_files(&paths, &formats).1
    } else if let Some(text) = &data.text {
        if cfg.ignore_blank && text.trim().is_empty() {
            return false;
        }
        if text.chars().count() as i64 > cfg.max_text_kb * 1024 || patterns.matches(&cfg.ignore_patterns, text) {
            return false;
        }
        // GNOME keeps only the paths as text once the file manager lets go of
        // copied files; that's the same copy, not a new one.
        if storage
            .history(Some(1))
            .first()
            .is_some_and(|top| top.kind == "files" && top.content == *text)
        {
            return false;
        }
        let url = is_url(text);
        if url && !cfg.store_url {
            return false;
        }
        if cfg.store_text || url {
            if echo.is_own_echo(&st::text_digest(text)) {
                return false;
            }
            storage.add_text(text, Some(&rich)).1
        } else if !rich.is_empty() {
            if echo.is_own_echo(&st::data_digest(&rich)) {
                return false;
            }
            storage.add_data(text, &describe(&rich), &rich).1
        } else {
            return false;
        }
    } else if let Some(pixbuf) = &data.image {
        let (width, height) = (pixbuf.width(), pixbuf.height());
        let digest = st::image_digest(&pixbuf.read_pixel_bytes(), width, height);
        if echo.is_own_echo(&digest) {
            return false;
        }
        if let Some(existing) = storage.find_digest(&digest) {
            storage.bump(existing);
            false
        } else {
            match pixbuf.save_to_bufferv("png", &[]) {
                Ok(png) if png.len() <= MAX_IMAGE_BYTES => storage.add_image(&png, width, height, &digest).1,
                _ => return false,
            }
        }
    } else if !rich.is_empty() {
        if echo.is_own_echo(&st::data_digest(&rich)) {
            return false;
        }
        storage.add_data("", &describe(&rich), &rich).1
    } else {
        return false;
    };
    if is_new {
        storage.trim(cfg.max_history);
    }
    true
}

// ---------------------------------------------------------------------------
// GTK side
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Request {
    Text,
    Image,
    Raw(String),
}

struct Offer {
    text: Option<String>,
    formats: Formats,
    pixbuf: Option<Pixbuf>,
}

pub struct Watcher {
    clipboard: gtk::Clipboard,
    settings: Box<dyn Fn() -> Settings>,
    storage: Rc<RefCell<Storage>>,
    on_changed: Box<dyn Fn()>,
    echo: RefCell<EchoGuard>,
    patterns: RefCell<PatternCache>,
    // gtk::Clipboard can only offer text or an image, so entries are put back
    // through the lower-level selection API on this widget.
    owner: gtk::Invisible,
    offer: RefCell<Option<Offer>>,
}

/// Collects the formats of one clipboard change, then saves one entry.
struct Capture {
    watcher: Rc<Watcher>,
    data: RefCell<CaptureData>,
    requests: Vec<Request>,
    pending: Cell<usize>,
    retried: RefCell<HashSet<String>>,
}

impl Capture {
    fn name(request: &Request) -> String {
        match request {
            Request::Text => "text".into(),
            Request::Image => "image".into(),
            Request::Raw(mime) => mime.clone(),
        }
    }

    fn issue(self: &Rc<Self>, request: &Request) {
        let clipboard = &self.watcher.clipboard;
        let this = self.clone();
        match request.clone() {
            Request::Text => clipboard.request_text(move |_, text| this.got_text(text.map(str::to_owned))),
            Request::Image => clipboard.request_image(move |_, pixbuf| this.got_image(pixbuf.cloned())),
            Request::Raw(mime) => {
                let atom = gdk::Atom::intern(&mime);
                clipboard.request_contents(&atom, move |_, data| {
                    let bytes = if data.length() > 0 { Some(data.data()) } else { None };
                    this.got_raw(mime.clone(), bytes);
                });
            }
        }
    }

    fn start(self: &Rc<Self>) {
        // Count first: GTK answers synchronously when we own the selection.
        self.pending.set(self.requests.len());
        // The first conversion requested from inside the TARGETS callback comes
        // back empty under XWayland, so ask from the main loop instead.
        let this = self.clone();
        glib::idle_add_local_once(move || {
            for request in &this.requests {
                this.issue(request);
            }
        });
    }

    /// Ask once more for a format that came back empty. True if retrying.
    fn retry(self: &Rc<Self>, request: Request) -> bool {
        if !self.retried.borrow_mut().insert(Self::name(&request)) {
            return false;
        }
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(60), move || this.issue(&request));
        true
    }

    fn done(&self) {
        self.pending.set(self.pending.get().saturating_sub(1));
        if self.pending.get() == 0 {
            let data = std::mem::take(&mut *self.data.borrow_mut());
            self.watcher.save(data);
        }
    }

    fn got_text(self: &Rc<Self>, text: Option<String>) {
        debug(|| {
            format!(
                "  text -> {:?}",
                text.as_ref().map(|t| t.chars().take(40).collect::<String>())
            )
        });
        if text.is_none() && self.retry(Request::Text) {
            return;
        }
        self.data.borrow_mut().text = text;
        self.done();
    }

    fn got_image(self: &Rc<Self>, pixbuf: Option<Pixbuf>) {
        if pixbuf.is_none() && self.retry(Request::Image) {
            return;
        }
        self.data.borrow_mut().image = pixbuf;
        self.done();
    }

    fn got_raw(self: &Rc<Self>, mime: String, bytes: Option<Vec<u8>>) {
        debug(|| format!("  {mime} -> {:?} bytes", bytes.as_ref().map(Vec::len)));
        if bytes.is_none() && self.retry(Request::Raw(mime.clone())) {
            return;
        }
        if let Some(bytes) = bytes {
            if bytes.len() <= MAX_FORMAT_BYTES {
                self.data.borrow_mut().raw.insert(mime, bytes);
            }
        }
        self.done();
    }
}

impl Watcher {
    pub fn new(
        settings: impl Fn() -> Settings + 'static,
        storage: Rc<RefCell<Storage>>,
        on_changed: impl Fn() + 'static,
    ) -> Rc<Watcher> {
        let owner = gtk::Invisible::new();
        owner.realize();
        let watcher = Rc::new(Watcher {
            clipboard: gtk::Clipboard::get(&gdk::SELECTION_CLIPBOARD),
            settings: Box::new(settings),
            storage,
            on_changed: Box::new(on_changed),
            echo: RefCell::new(EchoGuard::default()),
            patterns: RefCell::new(PatternCache::default()),
            owner,
            offer: RefCell::new(None),
        });
        // gtk-rs has no typed binding for "owner-change".
        let weak = Rc::downgrade(&watcher);
        watcher.clipboard.connect_local("owner-change", false, move |_values| {
            if let Some(watcher) = weak.upgrade() {
                // (no typed request_targets either: ask for TARGETS directly)
                let clipboard = watcher.clipboard.clone();
                clipboard.request_contents(&gdk::Atom::intern("TARGETS"), move |_, data| {
                    watcher.on_targets(&data.targets().unwrap_or_default());
                });
            }
            None
        });
        let weak = Rc::downgrade(&watcher);
        watcher.owner.connect_selection_get(move |_, data, info, _time| {
            if let Some(watcher) = weak.upgrade() {
                watcher.serve(data, info);
            }
        });
        watcher
    }

    fn on_targets(self: &Rc<Self>, atoms: &[gdk::Atom]) {
        let cfg = (self.settings)();
        let names: HashSet<String> = atoms.iter().map(|a| a.name().to_string()).collect();
        debug(|| format!("owner change, targets: {:?}", names));
        if cfg.ignore_secrets && SECRET_TARGETS.iter().any(|t| names.contains(*t)) {
            return;
        }
        let mut requests = Vec::new();
        let has_text = gtk::targets_include_text(atoms);
        let mut has_files = false;
        if cfg.store_files {
            for mime in FILE_TARGETS {
                if names.contains(*mime) {
                    requests.push(Request::Raw(mime.to_string()));
                    has_files = true;
                }
            }
        }
        if has_text {
            requests.push(Request::Text);
        }
        for (enabled, mimes) in [
            (cfg.store_html, HTML_TARGETS),
            (cfg.store_rtf, RTF_TARGETS),
            (cfg.store_pdf, PDF_TARGETS),
        ] {
            if enabled {
                requests.extend(
                    mimes
                        .iter()
                        .filter(|m| names.contains(**m))
                        .map(|m| Request::Raw(m.to_string())),
                );
            }
        }
        if !has_text && !has_files && cfg.store_images && gtk::targets_include_image(atoms, false) {
            requests.push(Request::Image);
        }
        if requests.is_empty() {
            return;
        }
        let capture = Rc::new(Capture {
            watcher: self.clone(),
            data: RefCell::new(CaptureData::default()),
            requests,
            pending: Cell::new(0),
            retried: RefCell::new(HashSet::new()),
        });
        capture.start();
    }

    fn save(&self, data: CaptureData) {
        let cfg = (self.settings)();
        let changed = store_capture(
            &cfg,
            &self.storage.borrow(),
            &mut self.echo.borrow_mut(),
            &mut self.patterns.borrow_mut(),
            data,
        );
        if changed {
            (self.on_changed)();
        }
    }

    // -- writing --------------------------------------------------------

    fn take_clipboard(&self, digests: HashSet<String>, offer: Offer) {
        self.echo.borrow_mut().set(digests);
        let selection = gdk::SELECTION_CLIPBOARD;
        gtk::selection_clear_targets(&self.owner, &selection);
        let mut targets: Vec<(String, u32)> = Vec::new();
        if offer.text.is_some() {
            targets.extend(TEXT_TARGETS.iter().map(|t| (t.to_string(), INFO_TEXT)));
        }
        targets.extend(offer.formats.keys().map(|m| (m.clone(), INFO_RAW)));
        if offer.pixbuf.is_some() {
            targets.extend(IMAGE_TARGETS.iter().map(|t| (t.to_string(), INFO_IMAGE)));
        }
        for (name, info) in targets {
            gtk::selection_add_target(&self.owner, &selection, &gdk::Atom::intern(&name), info);
        }
        *self.offer.borrow_mut() = Some(offer);
        gtk::selection_owner_set(Some(&self.owner), &selection, 0 /* GDK_CURRENT_TIME */);
    }

    fn serve(&self, data: &gtk::SelectionData, info: u32) {
        let offer = self.offer.borrow();
        let Some(offer) = offer.as_ref() else { return };
        match info {
            INFO_TEXT => {
                if let Some(text) = &offer.text {
                    data.set_text(text);
                }
            }
            INFO_IMAGE => {
                if let Some(pixbuf) = &offer.pixbuf {
                    data.set_pixbuf(pixbuf);
                }
            }
            _ => {
                let target = data.target();
                if let Some(bytes) = offer.formats.get(target.name().as_str()) {
                    data.set(&target, 8, bytes);
                }
            }
        }
    }

    pub fn set_text(&self, text: &str) {
        let digests = HashSet::from([st::text_digest(text)]);
        self.take_clipboard(
            digests,
            Offer {
                text: Some(text.to_owned()),
                formats: Formats::new(),
                pixbuf: None,
            },
        );
    }

    /// Put a history entry back on the clipboard; plain offers only its text.
    pub fn set_item(&self, item: &HistoryItem, plain: bool) -> Result<(), glib::Error> {
        if item.is_image() {
            let pixbuf = Pixbuf::from_file(self.storage.borrow().image_path(item))?;
            let digests = HashSet::from([item.digest.clone()]);
            self.take_clipboard(
                digests,
                Offer {
                    text: None,
                    formats: Formats::new(),
                    pixbuf: Some(pixbuf),
                },
            );
            return Ok(());
        }
        if plain {
            self.set_text(&item.content);
            return Ok(());
        }
        let text = (!item.content.is_empty()).then(|| item.content.clone());
        let mut digests = HashSet::from([item.digest.clone()]);
        if let Some(text) = &text {
            digests.insert(st::text_digest(text));
        }
        let formats = self.storage.borrow().formats(item.id);
        self.take_clipboard(
            digests,
            Offer {
                text,
                formats,
                pixbuf: None,
            },
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::tests::temp_storage;

    fn capture(
        cfg: &Settings,
        storage: &Storage,
        echo: &mut EchoGuard,
        text: Option<&str>,
        raw: &[(&str, &[u8])],
    ) -> bool {
        let data = CaptureData {
            text: text.map(str::to_owned),
            image: None,
            raw: raw.iter().map(|(m, d)| (m.to_string(), d.to_vec())).collect(),
        };
        store_capture(cfg, storage, echo, &mut PatternCache::default(), data)
    }

    fn saved(storage: &Storage) -> Vec<(String, String)> {
        storage.history(None).into_iter().map(|i| (i.kind, i.content)).collect()
    }

    #[test]
    fn ignore_patterns_skip_matches_and_survive_bad_regex() {
        let s = temp_storage("patterns");
        let cfg = Settings {
            ignore_patterns: vec![r"^\d{6}$".into(), "(unclosed".into()],
            ..Default::default()
        };
        let mut echo = EchoGuard::default();
        let mut patterns = PatternCache::default();
        for text in ["123456", "hello", "1234567"] {
            let data = CaptureData {
                text: Some(text.into()),
                ..Default::default()
            };
            store_capture(&cfg, &s, &mut echo, &mut patterns, data);
        }
        assert_eq!(
            saved(&s),
            [("text".into(), "1234567".into()), ("text".into(), "hello".into())]
        );
    }

    #[test]
    fn blank_and_own_echo_are_skipped() {
        let s = temp_storage("echo");
        let cfg = Settings::default();
        let mut echo = EchoGuard::default();
        assert!(!capture(&cfg, &s, &mut echo, Some("  \n "), &[]));
        echo.set(HashSet::from([st::text_digest("mine")]));
        assert!(!capture(&cfg, &s, &mut echo, Some("mine"), &[]));
        assert!(capture(&cfg, &s, &mut echo, Some("theirs"), &[]));
        assert_eq!(saved(&s), [("text".into(), "theirs".into())]);
    }

    #[test]
    fn rich_formats_are_kept_with_the_text() {
        let s = temp_storage("rich");
        capture(
            &Settings::default(),
            &s,
            &mut EchoGuard::default(),
            Some("bold"),
            &[("text/html", b"<b>bold</b>"), ("text/rtf", b"{\\rtf1 bold}")],
        );
        let item = &s.history(None)[0];
        assert_eq!((item.kind.as_str(), item.content.as_str()), ("text", "bold"));
        let formats = s.formats(item.id);
        assert_eq!(formats.len(), 2);
        assert_eq!(formats["text/html"], b"<b>bold</b>");
    }

    #[test]
    fn copied_files_become_one_files_entry() {
        let s = temp_storage("files");
        capture(
            &Settings::default(),
            &s,
            &mut EchoGuard::default(),
            Some("/tmp/a.txt\n/tmp/b c.png"),
            &[
                (
                    "x-special/gnome-copied-files",
                    b"cut\nfile:///tmp/a.txt\nfile:///tmp/b%20c.png",
                ),
                ("text/uri-list", b"file:///tmp/a.txt\r\nfile:///tmp/b%20c.png\r\n"),
            ],
        );
        let item = &s.history(None)[0];
        assert_eq!(item.kind, "files");
        assert_eq!(item.content, "/tmp/a.txt\n/tmp/b c.png");
        assert_eq!(item.summary, "2 files: a.txt, b c.png");
        // pasting from history must copy, never move, the files
        assert!(s.formats(item.id)["x-special/gnome-copied-files"].starts_with(b"copy\n"));
    }

    #[test]
    fn text_echoes_of_copied_files_and_rich_copies_add_nothing() {
        let s = temp_storage("echoes");
        let cfg = Settings::default();
        let mut echo = EchoGuard::default();
        capture(
            &cfg,
            &s,
            &mut echo,
            Some("/tmp/a.txt"),
            &[("text/uri-list", b"file:///tmp/a.txt")],
        );
        assert!(!capture(&cfg, &s, &mut echo, Some("/tmp/a.txt"), &[]));
        capture(&cfg, &s, &mut echo, Some("bold"), &[("text/html", b"<b>bold</b>")]);
        capture(&cfg, &s, &mut echo, Some("bold"), &[]);
        let items = s.history(None);
        assert_eq!(items.len(), 2);
        assert_eq!(s.formats(items[0].id)["text/html"], b"<b>bold</b>");
    }

    #[test]
    fn web_links_in_uri_list_are_not_files() {
        let raw: Formats = [("text/uri-list".to_string(), b"https://example.com/\r\n".to_vec())].into();
        assert!(parse_file_list(&raw).is_empty());
    }

    #[test]
    fn url_type_toggle() {
        let s = temp_storage("url");
        let cfg = Settings {
            store_url: false,
            ..Default::default()
        };
        let mut echo = EchoGuard::default();
        capture(&cfg, &s, &mut echo, Some("https://github.com/clipy/clipy"), &[]);
        capture(&cfg, &s, &mut echo, Some("not a link"), &[]);
        assert_eq!(saved(&s), [("text".into(), "not a link".into())]);
    }

    #[test]
    fn text_off_keeps_rich_version_only() {
        let s = temp_storage("textoff");
        let cfg = Settings {
            store_text: false,
            ..Default::default()
        };
        let mut echo = EchoGuard::default();
        capture(&cfg, &s, &mut echo, Some("plain only"), &[]);
        capture(&cfg, &s, &mut echo, Some("styled"), &[("text/html", b"<i>styled</i>")]);
        let items = s.history(None);
        assert_eq!(items.len(), 1);
        assert_eq!(
            (
                items[0].kind.as_str(),
                items[0].content.as_str(),
                items[0].summary.as_str()
            ),
            ("data", "styled", "HTML")
        );
    }

    #[test]
    fn pdf_without_text() {
        let s = temp_storage("pdf");
        capture(
            &Settings::default(),
            &s,
            &mut EchoGuard::default(),
            None,
            &[("application/pdf", b"%PDF-1.7")],
        );
        let item = &s.history(None)[0];
        assert_eq!((item.kind.as_str(), item.display_text()), ("data", "PDF document"));
    }

    #[test]
    fn images_are_saved_once() {
        let s = temp_storage("img");
        let pixbuf = Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 4, 3).unwrap();
        pixbuf.fill(0x3b5fc4ff);
        let mut echo = EchoGuard::default();
        let data = || CaptureData {
            image: Some(pixbuf.clone()),
            ..Default::default()
        };
        assert!(store_capture(
            &Settings::default(),
            &s,
            &mut echo,
            &mut PatternCache::default(),
            data()
        ));
        store_capture(
            &Settings::default(),
            &s,
            &mut echo,
            &mut PatternCache::default(),
            data(),
        );
        let items = s.history(None);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].summary, "Image 4×3");
    }
}
