//! User preferences, stored as JSON in ~/.config/clipnest/config.json.
//!
//! Only settings that differ from the defaults are written, so changing a
//! default in a new version reaches everyone who never touched it.

use std::cell::RefCell;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    // History
    pub max_history: i64, // pinned items don't count towards this
    pub reorder_on_select: bool,
    pub confirm_clear: bool,
    // Types to keep (the "Type" tab)
    pub store_text: bool,
    pub store_rtf: bool,
    pub store_pdf: bool,
    pub store_files: bool,
    pub store_url: bool,
    pub store_images: bool,
    pub store_html: bool,
    // Exclusions
    pub ignore_secrets: bool,
    pub ignore_blank: bool,
    pub ignore_patterns: Vec<String>,
    pub max_text_kb: i64,
    // Pasting
    pub auto_paste: bool,
    pub paste_keys: String, // ctrl+v | shift+insert | ctrl+shift+v
    pub paste_delay_ms: i64,
    // Menu (the "Menu" tab)
    pub menu_inline_items: i64,
    pub menu_folder_size: i64,
    pub menu_max_items: i64,
    pub menu_title_length: i64,
    pub menu_position: String, // "pointer" or "centre"
    pub menu_fixed_width: bool,
    pub menu_width: i64,
    pub menu_show_numbers: bool,
    pub menu_show_tooltips: bool,
    pub menu_show_images: bool,
    pub menu_thumb_width: i64,
    pub menu_thumb_height: i64,
    // Global shortcuts (applied to GNOME custom keybindings)
    pub hotkey_history: String,
    pub hotkey_search: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            max_history: 200,
            reorder_on_select: true,
            confirm_clear: true,
            store_text: true,
            store_rtf: true,
            store_pdf: true,
            store_files: true,
            store_url: true,
            store_images: true,
            store_html: true,
            ignore_secrets: true,
            ignore_blank: true,
            ignore_patterns: Vec::new(),
            max_text_kb: 1024,
            auto_paste: true,
            paste_keys: "ctrl+v".into(),
            paste_delay_ms: 180,
            menu_inline_items: 15,
            menu_folder_size: 10,
            menu_max_items: 50,
            menu_title_length: 20,
            menu_position: "pointer".into(),
            menu_fixed_width: false,
            menu_width: 360,
            menu_show_numbers: true,
            menu_show_tooltips: true,
            menu_show_images: true,
            menu_thumb_width: 100,
            menu_thumb_height: 32,
            hotkey_history: "<Super><Shift>v".into(),
            hotkey_search: "<Super><Shift>f".into(),
        }
    }
}

pub const PASTE_KEY_CHOICES: &[(&str, &str)] = &[
    ("ctrl+v", "Ctrl+V"),
    ("shift+insert", "Shift+Insert (works in terminals too)"),
    ("ctrl+shift+v", "Ctrl+Shift+V (terminals)"),
];

fn as_map(settings: &Settings) -> Map<String, Value> {
    match serde_json::to_value(settings) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

fn same_kind(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Bool(_), Value::Bool(_)) | (Value::String(_), Value::String(_)) => true,
        (Value::Number(x), Value::Number(y)) => x.is_i64() == y.is_i64(),
        (Value::Array(_), Value::Array(_)) => true,
        _ => false,
    }
}

type Listener = Box<dyn Fn(&str)>;

pub struct Config {
    path: PathBuf,
    settings: RefCell<Settings>,
    listeners: RefCell<Vec<Listener>>,
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> Config {
        let path = path.unwrap_or_else(|| paths::config_dir().join("config.json"));
        let settings = Self::read(&path);
        Config {
            path,
            settings: RefCell::new(settings),
            listeners: RefCell::new(Vec::new()),
        }
    }

    /// Unknown keys and values of the wrong type are ignored one by one.
    fn read(path: &PathBuf) -> Settings {
        let defaults = as_map(&Settings::default());
        let stored = match std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
        {
            Some(Value::Object(map)) => map,
            _ => return Settings::default(),
        };
        let mut merged = defaults.clone();
        for (key, default) in &defaults {
            if let Some(value) = stored.get(key) {
                if same_kind(value, default) {
                    let value = match value {
                        Value::Array(items) => Value::Array(items.iter().filter(|v| v.is_string()).cloned().collect()),
                        other => other.clone(),
                    };
                    merged.insert(key.clone(), value);
                }
            }
        }
        serde_json::from_value(Value::Object(merged)).unwrap_or_default()
    }

    fn save(&self) {
        let defaults = as_map(&Settings::default());
        let changed: Map<String, Value> = as_map(&self.settings.borrow())
            .into_iter()
            .filter(|(k, v)| defaults.get(k) != Some(v))
            .collect();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&Value::Object(changed)).unwrap_or_default();
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    pub fn get(&self) -> Settings {
        self.settings.borrow().clone()
    }

    /// Change settings; saves and tells listeners which keys changed.
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        let before = self.get();
        let mut after = before.clone();
        change(&mut after);
        if after == before {
            return;
        }
        let old = as_map(&before);
        let changed: Vec<String> = as_map(&after)
            .into_iter()
            .filter(|(k, v)| old.get(k) != Some(v))
            .map(|(k, _)| k)
            .collect();
        *self.settings.borrow_mut() = after;
        self.save();
        for key in &changed {
            for listener in self.listeners.borrow().iter() {
                listener(key);
            }
        }
    }

    /// Call listener(key) whenever a setting changes.
    pub fn connect(&self, listener: impl Fn(&str) + 'static) {
        self.listeners.borrow_mut().push(Box::new(listener));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("clipnest-test-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    #[test]
    fn bad_values_are_ignored_and_only_changes_are_saved() {
        let path = temp_path("config");
        std::fs::write(&path, r#"{"max_history": "lots", "auto_paste": false, "unknown": 1}"#).unwrap();
        let config = Config::load(Some(path.clone()));
        assert_eq!(config.get().max_history, 200);
        assert!(!config.get().auto_paste);

        let seen = Rc::new(Cell::new(0));
        let counter = seen.clone();
        config.connect(move |key| {
            assert_eq!(key, "max_history");
            counter.set(counter.get() + 1);
        });
        config.update(|s| s.max_history = 50);
        config.update(|s| s.max_history = 50); // unchanged: no second notification
        assert_eq!(seen.get(), 1);

        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let keys: Vec<&String> = saved.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["auto_paste", "max_history"]);
        assert_eq!(Config::load(Some(path)).get().max_history, 50);
    }

    #[test]
    fn patterns_keep_only_strings() {
        let path = temp_path("patterns");
        std::fs::write(&path, r#"{"ignore_patterns": ["^a$", 3, "b"]}"#).unwrap();
        assert_eq!(Config::load(Some(path)).get().ignore_patterns, vec!["^a$", "b"]);
    }
}
