//! What makes an open file's space its own: a colour, an emoji or symbol, a name. Saved per
//! file on this computer (a JSON file in the config folder, `localStorage` in the browser),
//! never in the `.fig`.
//!
//! A file is found again by its content key (`Tab::doc_key`); a new copy of it (another key,
//! same file name) gets the style last used under that name.

use serde::{Deserialize, Serialize};

use crate::icons::Icon;
use crate::theme::Colour;

/// Most files remembered; the least recently used go first.
const MAX_ENTRIES: usize = 500;
/// Longest name or emoji kept, in characters.
const MAX_NAME: usize = 64;
const MAX_EMOJI: usize = 8;

/// A space's look. `None` fields keep the defaults: the tint picked from the file name, a
/// plain dot, the file name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<Colour>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<Symbol>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Style {
    /// Trims what the user typed: an empty name or emoji means the default.
    pub fn tidy(mut self) -> Self {
        self.name = self
            .name
            .map(|n| n.trim().chars().take(MAX_NAME).collect::<String>())
            .filter(|n| !n.is_empty());
        self.symbol = match self.symbol {
            Some(Symbol::Emoji(e)) => {
                let e: String = e.trim().chars().take(MAX_EMOJI).collect();
                (!e.is_empty()).then_some(Symbol::Emoji(e))
            }
            other => other,
        };
        self
    }
}

/// The badge of a space: an emoji (typed or picked) or one of the app's icons.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symbol {
    Emoji(String),
    Icon(String),
}

/// Emoji offered in the space editor (all drawn by egui's built-in emoji font).
pub const EMOJI: [&str; 16] = [
    "🎨", "📱", "💻", "🚀", "⭐", "🔥", "🌈", "🌿", "💡", "📐", "🎯", "📦", "❤", "🌙", "⚡", "🍀",
];

/// The hint in the editor's "type an emoji" field.
pub const EMOJI_HINT: &str = "😀";

/// Icons offered in the space editor, with the names they are saved under.
pub const ICONS: [(&str, Icon); 8] = [
    ("frame", Icon::Frame),
    ("component", Icon::Component),
    ("page", Icon::Page),
    ("text", Icon::Text),
    ("image", Icon::Image),
    ("ellipse", Icon::Ellipse),
    ("vector", Icon::Vector),
    ("section", Icon::Section),
];

/// The icon saved as `name`, if it is one of [`ICONS`].
pub fn icon(name: &str) -> Option<Icon> {
    ICONS.iter().find(|(n, _)| *n == name).map(|(_, i)| *i)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    /// The file's content key, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    /// The file name, for a new copy of the same file.
    file: String,
    style: Style,
}

/// Where the styles live.
#[derive(Debug)]
enum Backend {
    /// Tests and headless runs: nothing is read or written.
    Memory,
    #[cfg(not(target_arch = "wasm32"))]
    File(std::path::PathBuf),
    #[cfg(target_arch = "wasm32")]
    LocalStorage,
}

/// The remembered styles, most recently used last.
#[derive(Debug)]
pub struct Store {
    entries: Vec<Entry>,
    backend: Backend,
}

impl Default for Store {
    fn default() -> Self {
        Self::memory()
    }
}

#[cfg(target_arch = "wasm32")]
const STORAGE_KEY: &str = "figstudio.spaces";

impl Store {
    /// A store that only lives in memory.
    pub fn memory() -> Self {
        Self {
            entries: Vec::new(),
            backend: Backend::Memory,
        }
    }

    /// The store of this computer: `<config>/figstudio/spaces.json` on the desktop,
    /// `localStorage` in the browser. Unreadable or damaged data starts an empty store.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match dirs::config_dir() {
                Some(dir) => Self::at(dir.join("figstudio").join("spaces.json")),
                None => Self::memory(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let text = local_storage().and_then(|s| s.get_item(STORAGE_KEY).ok().flatten());
            Self {
                entries: text.as_deref().map(parse).unwrap_or_default(),
                backend: Backend::LocalStorage,
            }
        }
    }

    /// The store kept in the file at `path` (read now; written on every change).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn at(path: std::path::PathBuf) -> Self {
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::warn!("space styles not read: {e}");
                Vec::new()
            }
        };
        Self {
            entries,
            backend: Backend::File(path),
        }
    }

    /// The style saved for the file with content `key` named `file`: by key first, else the
    /// last one saved under that name.
    pub fn lookup(&self, key: Option<&str>, file: &str) -> Option<Style> {
        let by_key = key.and_then(|k| {
            self.entries
                .iter()
                .rev()
                .find(|e| e.key.as_deref() == Some(k))
        });
        by_key
            .or_else(|| self.entries.iter().rev().find(|e| e.file == file))
            .map(|e| e.style.clone())
    }

    /// Saves `style` for the file (`key`, `file`). The default style forgets the file.
    pub fn remember(&mut self, key: Option<&str>, file: &str, style: &Style) {
        let style = style.clone().tidy();
        self.entries.retain(|e| match (key, e.key.as_deref()) {
            (Some(k), Some(ek)) => k != ek,
            _ => e.file != file,
        });
        if style != Style::default() {
            self.entries.push(Entry {
                key: key.map(str::to_owned),
                file: file.to_owned(),
                style,
            });
        }
        let extra = self.entries.len().saturating_sub(MAX_ENTRIES);
        self.entries.drain(..extra);
        self.save();
    }

    fn save(&self) {
        let text = match serde_json::to_string_pretty(&self.entries) {
            Ok(text) => text,
            Err(e) => {
                tracing::warn!("space styles not saved: {e}");
                return;
            }
        };
        match &self.backend {
            Backend::Memory => {}
            #[cfg(not(target_arch = "wasm32"))]
            Backend::File(path) => {
                let written = path
                    .parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(path, text));
                if let Err(e) = written {
                    tracing::warn!("space styles not saved: {e}");
                }
            }
            #[cfg(target_arch = "wasm32")]
            Backend::LocalStorage => {
                if let Some(storage) = local_storage() {
                    let _ = storage.set_item(STORAGE_KEY, &text);
                }
            }
        }
    }
}

/// The entries in `text`, or none when it is not a valid list (a damaged file starts over).
fn parse(text: &str) -> Vec<Entry> {
    serde_json::from_str(text).unwrap_or_else(|e| {
        tracing::warn!("space styles ignored: {e}");
        Vec::new()
    })
}

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rose() -> Style {
        Style {
            colour: Some(Colour::Preset(4)),
            symbol: Some(Symbol::Emoji("🚀".into())),
            name: Some("Launch".into()),
        }
    }

    #[test]
    fn a_file_is_found_by_its_key_then_by_its_name() {
        let mut store = Store::memory();
        store.remember(Some("k1"), "a.fig", &rose());
        assert_eq!(
            store.lookup(Some("k1"), "renamed.fig"),
            Some(rose()),
            "by key"
        );
        assert_eq!(
            store.lookup(Some("k2"), "a.fig"),
            Some(rose()),
            "a new copy, same name"
        );
        assert_eq!(store.lookup(Some("k3"), "b.fig"), None);
        let blue = Style {
            colour: Some(Colour::Custom([0, 0, 255])),
            ..Style::default()
        };
        store.remember(Some("k1"), "a.fig", &blue);
        assert_eq!(store.lookup(Some("k1"), "a.fig"), Some(blue), "replaced");
        assert_eq!(store.entries.len(), 1);
        store.remember(Some("k1"), "a.fig", &Style::default());
        assert_eq!(
            store.lookup(Some("k1"), "a.fig"),
            None,
            "the default forgets"
        );
        assert!(store.entries.is_empty());
    }

    #[test]
    fn typed_names_and_emoji_are_tidied() {
        let style = Style {
            colour: None,
            symbol: Some(Symbol::Emoji("   ".into())),
            name: Some(format!("  {}  ", "x".repeat(100))),
        }
        .tidy();
        assert_eq!(style.symbol, None, "blank emoji");
        assert_eq!(style.name.map(|n| n.chars().count()), Some(MAX_NAME));
        assert_eq!(
            Style {
                name: Some(" ".into()),
                ..Style::default()
            }
            .tidy(),
            Style::default()
        );
    }

    #[test]
    fn the_store_survives_a_restart_and_a_damaged_file() {
        let dir = std::env::temp_dir().join(format!("figstudio-spaces-{}", std::process::id()));
        let path = dir.join("nested").join("spaces.json");
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::at(path.clone());
        assert!(store.entries.is_empty(), "no file yet");
        store.remember(Some("k1"), "a.fig", &rose());
        assert_eq!(
            Store::at(path.clone()).lookup(Some("k1"), "a.fig"),
            Some(rose())
        );
        std::fs::write(&path, "{ not json").expect("write");
        let damaged = Store::at(path.clone());
        assert!(damaged.entries.is_empty(), "starts over");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_files_are_forgotten_first() {
        let mut store = Store::memory();
        for i in 0..MAX_ENTRIES + 3 {
            store.remember(Some(&format!("k{i}")), &format!("{i}.fig"), &rose());
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        assert_eq!(store.lookup(Some("k0"), "0.fig"), None);
        assert!(store.lookup(Some("k3"), "3.fig").is_some());
    }

    #[test]
    fn every_offered_emoji_and_icon_can_be_drawn() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let _ = ctx.run_ui(Default::default(), |_| {});
        let font = egui::FontId::proportional(14.0);
        ctx.fonts_mut(|fonts| {
            // `has_glyph` misses egui's emoji font here; compare with the box drawn for a
            // character no font has instead.
            let mut drawn = |s: &str| {
                let galley = fonts.layout_no_wrap(s.into(), font.clone(), egui::Color32::WHITE);
                galley.rows[0].glyphs[0].uv_rect
            };
            let tofu = drawn("\u{E000}");
            assert_eq!(drawn("\u{1F9E9}"), tofu, "the check sees a missing emoji");
            for e in EMOJI.into_iter().chain([EMOJI_HINT]) {
                assert_ne!(drawn(e), tofu, "{e}");
            }
        });
        for (name, i) in ICONS {
            assert_eq!(icon(name), Some(i));
        }
        assert_eq!(icon("nope"), None);
    }
}
