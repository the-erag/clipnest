//! SQLite storage for the clipboard history.
//!
//! Images are kept as PNG files next to the database; the history row only
//! stores the file name. Other clipboard formats of an entry (HTML, RTF, PDF,
//! file lists) live in the formats table, so pasting restores them too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::paths;
use crate::util::now;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,                -- 'text', 'image', 'files' or 'data'
    content TEXT NOT NULL,             -- text, PNG file name, or file paths (one per line)
    digest TEXT NOT NULL UNIQUE,
    summary TEXT NOT NULL DEFAULT '',  -- e.g. \"Image 640×480\"
    created REAL NOT NULL,
    pinned INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS history_created ON history(created DESC);

CREATE TABLE IF NOT EXISTS formats (
    item_id INTEGER NOT NULL REFERENCES history(id) ON DELETE CASCADE,
    mime TEXT NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY (item_id, mime)
);
";

pub type Formats = HashMap<String, Vec<u8>>;

fn sha1_hex(parts: &[&[u8]]) -> String {
    let mut h = sha1_smol::Sha1::new();
    for part in parts {
        h.update(part);
    }
    h.digest().to_string()
}

pub fn text_digest(text: &str) -> String {
    format!("t:{}", sha1_hex(&[text.as_bytes()]))
}

pub fn image_digest(pixels: &[u8], width: i32, height: i32) -> String {
    format!("i:{}", sha1_hex(&[pixels, format!("{width}x{height}").as_bytes()]))
}

pub fn files_digest(paths: &[String]) -> String {
    format!("f:{}", sha1_hex(&[paths.join("\n").as_bytes()]))
}

pub fn data_digest(formats: &Formats) -> String {
    let mut mimes: Vec<&String> = formats.keys().collect();
    mimes.sort();
    let mut parts: Vec<&[u8]> = Vec::new();
    for mime in mimes {
        parts.push(mime.as_bytes());
        parts.push(&formats[mime]);
    }
    format!("d:{}", sha1_hex(&parts))
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryItem {
    pub id: i64,
    pub kind: String,
    pub content: String,
    pub digest: String,
    pub summary: String,
    pub created: f64,
    pub pinned: bool,
}

impl HistoryItem {
    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }

    /// What menus show and search looks at.
    pub fn display_text(&self) -> &str {
        if self.kind == "image" || self.kind == "files" || self.content.is_empty() {
            &self.summary
        } else {
            &self.content
        }
    }
}

const COLUMNS: &str = "id, kind, content, digest, summary, created, pinned";

fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<HistoryItem> {
    Ok(HistoryItem {
        id: row.get(0)?,
        kind: row.get(1)?,
        content: row.get(2)?,
        digest: row.get(3)?,
        summary: row.get(4)?,
        created: row.get(5)?,
        pinned: row.get::<_, i64>(6)? != 0,
    })
}

/// Log database errors instead of taking the whole app down.
fn ok<T: Default>(result: rusqlite::Result<T>) -> T {
    result.unwrap_or_else(|err| {
        eprintln!("clipnest: database error: {err}");
        T::default()
    })
}

pub struct Storage {
    pub directory: PathBuf,
    image_dir: PathBuf,
    db: Connection,
}

impl Storage {
    pub fn open(directory: Option<PathBuf>) -> rusqlite::Result<Storage> {
        let directory = directory.unwrap_or_else(paths::data_dir);
        let image_dir = directory.join("images");
        let _ = std::fs::create_dir_all(&image_dir);
        let db = Connection::open(directory.join("clipnest.db"))?;
        db.pragma_update(None, "foreign_keys", "ON")?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.execute_batch(SCHEMA)?;
        Ok(Storage {
            directory,
            image_dir,
            db,
        })
    }

    pub fn history(&self, limit: Option<usize>) -> Vec<HistoryItem> {
        let sql = format!(
            "SELECT {COLUMNS} FROM history ORDER BY created DESC, id DESC LIMIT {}",
            limit.map(|l| l as i64).unwrap_or(-1)
        );
        ok((|| {
            let mut stmt = self.db.prepare(&sql)?;
            let rows = stmt.query_map([], row_to_item)?;
            rows.collect()
        })())
    }

    pub fn get(&self, id: i64) -> Option<HistoryItem> {
        ok(self
            .db
            .query_row(
                &format!("SELECT {COLUMNS} FROM history WHERE id = ?"),
                [id],
                row_to_item,
            )
            .optional())
    }

    pub fn find_digest(&self, digest: &str) -> Option<i64> {
        ok(self
            .db
            .query_row("SELECT id FROM history WHERE digest = ?", [digest], |r| r.get(0))
            .optional())
    }

    /// Insert a new entry, or move an identical existing one to the top.
    /// `formats` are merged into the entry's extra formats: a re-copy brings
    /// the newest version of each, and a plain-text-only echo of the same
    /// content (GNOME keeps just the text when the copying app quits) doesn't
    /// throw the rich versions away. Returns (item id, is new).
    fn insert(&self, kind: &str, content: &str, digest: &str, summary: &str, formats: Option<&Formats>) -> (i64, bool) {
        let (id, is_new) = match self.find_digest(digest) {
            Some(id) => {
                ok(self
                    .db
                    .execute("UPDATE history SET created = ? WHERE id = ?", params![now(), id]));
                (id, false)
            }
            None => {
                ok(self.db.execute(
                    "INSERT INTO history (kind, content, digest, summary, created) VALUES (?, ?, ?, ?, ?)",
                    params![kind, content, digest, summary, now()],
                ));
                (self.db.last_insert_rowid(), true)
            }
        };
        if let Some(formats) = formats {
            for (mime, data) in formats {
                ok(self.db.execute(
                    "INSERT OR REPLACE INTO formats (item_id, mime, data) VALUES (?, ?, ?)",
                    params![id, mime, data],
                ));
            }
        }
        (id, is_new)
    }

    pub fn add_text(&self, text: &str, formats: Option<&Formats>) -> (i64, bool) {
        self.insert("text", text, &text_digest(text), "", formats)
    }

    /// The PNG is written once per unique image.
    pub fn add_image(&self, png: &[u8], width: i32, height: i32, digest: &str) -> (i64, bool) {
        let filename = format!("{}.png", digest.replace(':', "_"));
        let target = self.image_dir.join(&filename);
        if !target.exists() {
            let tmp = target.with_extension("tmp");
            if std::fs::write(&tmp, png).is_ok() {
                let _ = std::fs::rename(&tmp, &target);
            }
        }
        self.insert("image", &filename, digest, &format!("Image {width}×{height}"), None)
    }

    /// A copied set of files: paths for display, formats to paste them again.
    pub fn add_files(&self, paths: &[String], formats: &Formats) -> (i64, bool) {
        let names: Vec<String> = paths
            .iter()
            .map(|p| {
                Path::new(p.trim_end_matches('/'))
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| p.clone())
            })
            .collect();
        let summary = if names.len() == 1 {
            names[0].clone()
        } else {
            format!("{} files: {}", names.len(), names.join(", "))
        };
        self.insert(
            "files",
            &paths.join("\n"),
            &files_digest(paths),
            &summary,
            Some(formats),
        )
    }

    /// Rich content saved without its plain text (e.g. only HTML or PDF).
    pub fn add_data(&self, text: &str, summary: &str, formats: &Formats) -> (i64, bool) {
        self.insert("data", text, &data_digest(formats), summary, Some(formats))
    }

    pub fn formats(&self, id: i64) -> Formats {
        ok((|| {
            let mut stmt = self.db.prepare("SELECT mime, data FROM formats WHERE item_id = ?")?;
            let rows = stmt.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
            rows.collect()
        })())
    }

    pub fn image_path(&self, item: &HistoryItem) -> PathBuf {
        self.image_dir.join(&item.content)
    }

    pub fn bump(&self, id: i64) {
        ok(self
            .db
            .execute("UPDATE history SET created = ? WHERE id = ?", params![now(), id]));
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) {
        ok(self
            .db
            .execute("UPDATE history SET pinned = ? WHERE id = ?", params![pinned as i64, id]));
    }

    fn remove_files(&self, rows: &[(String, String)]) {
        for (kind, content) in rows {
            if kind == "image" {
                let _ = std::fs::remove_file(self.image_dir.join(content));
            }
        }
    }

    fn kinds_and_contents(&self, sql: &str, args: &[i64]) -> Vec<(String, String)> {
        ok((|| {
            let mut stmt = self.db.prepare(sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(args), |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })())
    }

    pub fn delete(&self, id: i64) {
        let rows = self.kinds_and_contents("SELECT kind, content FROM history WHERE id = ?", &[id]);
        ok(self.db.execute("DELETE FROM history WHERE id = ?", [id]));
        self.remove_files(&rows);
    }

    pub fn clear(&self, keep_pinned: bool) {
        let filter = if keep_pinned { "WHERE pinned = 0" } else { "" };
        let rows = self.kinds_and_contents(&format!("SELECT kind, content FROM history {filter}"), &[]);
        ok(self.db.execute(&format!("DELETE FROM history {filter}"), []));
        self.remove_files(&rows);
    }

    /// Drop the oldest unpinned entries beyond max_items. Returns how many went.
    pub fn trim(&self, max_items: i64) -> usize {
        let doomed: Vec<(i64, String, String)> = ok((|| {
            let mut stmt = self.db.prepare(
                "SELECT id, kind, content FROM history WHERE pinned = 0 \
                 ORDER BY created DESC, id DESC LIMIT -1 OFFSET ?",
            )?;
            let rows = stmt.query_map([max_items.max(0)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect()
        })());
        for (id, _, _) in &doomed {
            ok(self.db.execute("DELETE FROM history WHERE id = ?", [id]));
        }
        let files: Vec<(String, String)> = doomed.into_iter().map(|(_, k, c)| (k, c)).collect();
        self.remove_files(&files);
        files.len()
    }

    pub fn count(&self) -> i64 {
        ok(self.db.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0)))
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn temp_storage(name: &str) -> Storage {
        let dir = std::env::temp_dir().join(format!("clipnest-db-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Storage::open(Some(dir)).unwrap()
    }

    fn contents(s: &Storage) -> Vec<String> {
        s.history(None).into_iter().map(|i| i.content).collect()
    }

    #[test]
    fn newest_first_and_duplicates_move_to_top() {
        let s = temp_storage("order");
        s.add_text("one", None);
        s.add_text("two", None);
        let (_, is_new) = s.add_text("one", None);
        assert!(!is_new);
        assert_eq!(contents(&s), ["one", "two"]);
    }

    #[test]
    fn trim_and_clear_keep_pinned_items() {
        let s = temp_storage("trim");
        let (first, _) = s.add_text("keep me", None);
        s.set_pinned(first, true);
        for n in 0..5 {
            s.add_text(&format!("item {n}"), None);
        }
        assert_eq!(s.trim(2), 3);
        assert_eq!(contents(&s), ["item 4", "item 3", "keep me"]);
        s.clear(true);
        assert_eq!(contents(&s), ["keep me"]);
        s.clear(false);
        assert!(contents(&s).is_empty());
    }

    #[test]
    fn image_files_go_with_their_entry() {
        let s = temp_storage("image");
        let digest = image_digest(&[0u8; 12], 2, 2);
        let (id, is_new) = s.add_image(b"fake-png", 2, 2, &digest);
        assert!(is_new);
        let item = s.get(id).unwrap();
        assert_eq!(item.summary, "Image 2×2");
        let path = s.image_path(&item);
        assert!(path.exists());
        s.delete(id);
        assert!(!path.exists());
    }

    #[test]
    fn formats_are_merged_on_recopy_and_deleted_with_the_entry() {
        let s = temp_storage("formats");
        let html = |v: &[u8]| Formats::from([("text/html".to_string(), v.to_vec())]);
        let (id, _) = s.add_text("x", Some(&html(b"<b>x</b>")));
        s.add_text("x", Some(&Formats::new())); // plain-text echo keeps the HTML
        assert_eq!(s.formats(id), html(b"<b>x</b>"));
        s.add_text("x", Some(&html(b"<i>x</i>"))); // a newer rich copy wins
        assert_eq!(s.formats(id), html(b"<i>x</i>"));
        s.delete(id);
        assert!(s.formats(id).is_empty());
    }

    #[test]
    fn digests_match_the_python_version() {
        // Same format as the first (Python) version, so existing history keeps deduplicating.
        assert_eq!(text_digest("hello"), "t:aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }
}
