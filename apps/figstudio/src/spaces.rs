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
/// Longest emoji kept, in characters: a family of four with skin tones is 11.
const MAX_EMOJI: usize = 16;

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
                // One grapheme: a badge shows one emoji, not a word.
                use unicode_segmentation::UnicodeSegmentation as _;
                // Whole or not at all: cutting a long grapheme (a family with skin tones
                // is one) would leave a broken sequence.
                let first = e.trim().graphemes(true).next().unwrap_or("");
                (!first.is_empty() && first.chars().count() <= MAX_EMOJI)
                    .then(|| Symbol::Emoji(first.to_owned()))
            }
            Some(Symbol::Icon(name)) => icon(&name).is_some().then_some(Symbol::Icon(name)),
            None => None,
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

/// Which file a style belongs to: its content key when known, its name, and a hash of its
/// folder (never the path itself), so a same-named file in another folder is another file.
#[derive(Clone, Copy, Debug)]
pub struct FileId<'a> {
    pub key: Option<&'a str>,
    pub file: &'a str,
    pub folder: Option<&'a str>,
}

impl<'a> FileId<'a> {
    pub fn of(tab: &'a crate::tab::Tab, title: &'a str) -> Self {
        Self {
            key: tab.doc_key.as_deref(),
            file: title,
            folder: tab.folder.as_deref(),
        }
    }
}

/// A short hash of the folder holding `path` (`None` without one, as in the browser).
pub fn folder_of(path: &std::path::Path) -> Option<String> {
    use sha2::Digest as _;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    let hash = sha2::Sha256::digest(parent.to_string_lossy().as_bytes());
    Some(hash[..8].iter().map(|b| format!("{b:02x}")).collect())
}

/// One remembered file. A default `style` is kept too (a reset), so a reset file does not
/// fall back to an older copy's style.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    /// The file's content key, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    /// The file name, for a new copy of the same file.
    file: String,
    /// [`folder_of`] the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
    style: Style,
}

impl Entry {
    /// Cuts what a damaged or hostile store could hold: long keys, names and folders.
    fn tidy(mut self) -> Option<Self> {
        let short = |s: &str, max: usize| s.chars().count() <= max;
        if !short(&self.file, MAX_FILE)
            || self.key.as_deref().is_some_and(|k| !short(k, MAX_KEY))
            || self.folder.as_deref().is_some_and(|f| !short(f, MAX_KEY))
        {
            return None;
        }
        self.style = self.style.tidy();
        Some(self)
    }

    fn is(&self, id: &FileId<'_>) -> bool {
        match id.key {
            Some(key) => self.key.as_deref() == Some(key),
            None => {
                self.key.is_none() && self.file == id.file && self.folder.as_deref() == id.folder
            }
        }
    }
}

/// Longest file name, key or folder hash kept from the store.
const MAX_FILE: usize = 255;
const MAX_KEY: usize = 64;
/// Largest store read (bytes); a bigger one is ignored, not read.
const MAX_BYTES: u64 = 2 << 20;

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
    /// `localStorage` in the browser. Unreadable data starts an empty store; damaged entries
    /// are skipped one by one.
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
            // The browser hands over the whole string; one too big (or storage that refuses
            // to be read) is kept as it is, and this session's styles stay in memory.
            match local_storage().map(|s| s.get_item(STORAGE_KEY)) {
                Some(Ok(text)) if text.as_ref().is_none_or(|t| t.len() as u64 <= MAX_BYTES) => {
                    Self {
                        entries: text.as_deref().map(parse).unwrap_or_default(),
                        backend: Backend::LocalStorage,
                    }
                }
                _ => {
                    tracing::warn!("space styles not read, kept in memory this time");
                    Self::memory()
                }
            }
        }
    }

    /// The store kept in the file at `path` (read now; written on every change). A file
    /// that cannot be read (too big, no permission) is left as it is: this session keeps its
    /// styles in memory only, so what was not read is never written over.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn at(path: std::path::PathBuf) -> Self {
        use std::io::Read as _;
        // Capped while reading, not by an earlier size: the file may grow in between.
        let read = std::fs::File::open(&path).and_then(|file| {
            let mut bytes = Vec::new();
            file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > MAX_BYTES {
                return Err(std::io::Error::other(format!(
                    "more than {MAX_BYTES} bytes"
                )));
            }
            Ok(bytes)
        });
        match read {
            // Not text or not JSON: nothing in it can be used, so it is started over.
            Ok(bytes) => Self {
                entries: std::str::from_utf8(&bytes).map(parse).unwrap_or_default(),
                backend: Backend::File(path),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self {
                entries: Vec::new(),
                backend: Backend::File(path),
            },
            Err(e) => {
                tracing::warn!("space styles not read, kept in memory this time: {e}");
                Self::memory()
            }
        }
    }

    /// The style saved for the file: by its content key first, else the last one saved
    /// under the same name in the same folder, else under that name with no folder (saved
    /// before folders were). A reset file gives the default style.
    pub fn lookup(&self, id: FileId<'_>) -> Option<Style> {
        let newest = |keep: &dyn Fn(&Entry) -> bool| self.entries.iter().rev().find(|e| keep(e));
        id.key
            .and_then(|k| newest(&|e| e.key.as_deref() == Some(k)))
            .or_else(|| newest(&|e| e.file == id.file && e.folder.as_deref() == id.folder))
            .or_else(|| newest(&|e| e.file == id.file && e.folder.is_none()))
            .map(|e| e.style.clone())
    }

    /// Saves `style` for the file; the default style is saved too (a reset sticks).
    pub fn remember(&mut self, id: FileId<'_>, style: &Style) {
        self.entries.retain(|e| !e.is(&id));
        self.entries.push(Entry {
            key: id.key.map(str::to_owned),
            file: id.file.to_owned(),
            folder: id.folder.map(str::to_owned),
            style: style.clone().tidy(),
        });
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
                if let Err(e) = write_private(path, &text) {
                    tracing::warn!("space styles not saved: {e}");
                }
            }
            #[cfg(target_arch = "wasm32")]
            Backend::LocalStorage => {
                let set = local_storage().map(|s| s.set_item(STORAGE_KEY, &text));
                if !matches!(set, Some(Ok(()))) {
                    tracing::warn!("space styles not saved: the browser's storage refused them");
                }
            }
        }
    }
}

/// Writes `text` to `path` all or nothing: into a new, uniquely named temporary file next
/// to it, then renamed over it, so a crash or a second figstudio writing at the same time
/// never leaves half a file. On Unix only its owner can read it (the temporary file is
/// created that way, and the rename keeps it); Windows sets no ACL here.
#[cfg(not(target_arch = "wasm32"))]
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::Builder::new()
        .prefix(".spaces-")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The entries in `text`: each damaged or oversized one is skipped, not the whole list, and
/// at most the newest [`MAX_ENTRIES`] are kept.
fn parse(text: &str) -> Vec<Entry> {
    let items: Vec<serde_json::Value> = serde_json::from_str(text).unwrap_or_else(|e| {
        tracing::warn!("space styles ignored: {e}");
        Vec::new()
    });
    let mut entries: Vec<Entry> = items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<Entry>(item).ok())
        .filter_map(Entry::tidy)
        .collect();
    let extra = entries.len().saturating_sub(MAX_ENTRIES);
    entries.drain(..extra);
    entries
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

    fn id<'a>(key: Option<&'a str>, file: &'a str, folder: Option<&'a str>) -> FileId<'a> {
        FileId { key, file, folder }
    }

    #[test]
    fn a_long_emoji_is_kept_whole_or_not_at_all() {
        let emoji = |e: &str| {
            Style {
                symbol: Some(Symbol::Emoji(e.into())),
                ..Style::default()
            }
            .tidy()
            .symbol
        };
        // A family of four with skin tones: one grapheme of 11 characters.
        let family = "👨🏽\u{200d}👩🏽\u{200d}👧🏽\u{200d}👦🏽";
        assert_eq!(family.chars().count(), 11);
        assert_eq!(
            emoji(&format!("{family}xyz")),
            Some(Symbol::Emoji(family.into()))
        );
        // Longer than any emoji worth drawing: dropped, never cut into a broken sequence.
        let silly = format!("a{}", "\u{301}".repeat(30));
        assert_eq!(emoji(&silly), None);
    }

    #[test]
    fn a_file_is_found_by_its_key_then_by_its_name_in_the_same_folder() {
        let mut store = Store::memory();
        store.remember(id(Some("k1"), "a.fig", Some("f1")), &rose());
        let found = |s: &Store, key, file, folder| s.lookup(id(key, file, folder));
        assert_eq!(
            found(&store, Some("k1"), "renamed.fig", Some("f9")),
            Some(rose()),
            "by key"
        );
        assert_eq!(
            found(&store, Some("k2"), "a.fig", Some("f1")),
            Some(rose()),
            "a new copy"
        );
        assert_eq!(
            found(&store, Some("k2"), "a.fig", Some("f2")),
            None,
            "a same-named file in another folder is another file"
        );
        assert_eq!(found(&store, Some("k3"), "b.fig", Some("f1")), None);
        let blue = Style {
            colour: Some(Colour::Custom([0, 0, 255])),
            ..Style::default()
        };
        store.remember(id(Some("k1"), "a.fig", Some("f1")), &blue);
        assert_eq!(
            found(&store, Some("k1"), "a.fig", Some("f1")),
            Some(blue),
            "replaced"
        );
        assert_eq!(store.entries.len(), 1);
    }

    #[test]
    fn a_reset_sticks_even_when_an_older_copy_has_a_style() {
        let mut store = Store::memory();
        let (a, b) = (
            id(Some("kA"), "design.fig", Some("f")),
            id(Some("kB"), "design.fig", Some("f")),
        );
        store.remember(a, &rose());
        assert_eq!(store.lookup(b), Some(rose()), "the re-export inherits");
        store.remember(b, &rose());
        store.remember(b, &Style::default());
        assert_eq!(store.lookup(b), Some(Style::default()), "the reset stays");
        assert_eq!(
            store.lookup(a),
            Some(rose()),
            "the older copy keeps its own"
        );
        // Without a key only the unkeyed entry of that name and folder is replaced.
        store.remember(id(None, "design.fig", Some("f")), &Style::default());
        assert_eq!(store.lookup(a), Some(rose()), "keyed entries stay");
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
        let one = |typed: &str| {
            Style {
                symbol: Some(Symbol::Emoji(typed.into())),
                ..Style::default()
            }
            .tidy()
            .symbol
        };
        assert_eq!(
            one("dddddddd"),
            Some(Symbol::Emoji("d".into())),
            "one character"
        );
        assert_eq!(
            one("👍🏽🚀"),
            Some(Symbol::Emoji("👍🏽".into())),
            "a skin tone stays"
        );
        assert_eq!(
            one(" 🇯🇵"),
            Some(Symbol::Emoji("🇯🇵".into())),
            "a flag is one"
        );
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
        let a = id(Some("k1"), "a.fig", None);
        store.remember(a, &rose());
        assert_eq!(Store::at(path.clone()).lookup(a), Some(rose()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .expect("written")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "only its owner reads it");
        }
        let left = |dir: &std::path::Path| {
            std::fs::read_dir(dir)
                .expect("dir")
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != "spaces.json" && n != "spaces.json.tmp")
                .collect::<Vec<_>>()
        };
        assert!(
            left(path.parent().expect("dir")).is_empty(),
            "no temporary file left"
        );
        // A temporary file an older version left behind, readable by anyone, changes
        // nothing: the store is written through a new one, still owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let stale = path.with_extension("json.tmp");
            std::fs::write(&stale, "[]").expect("write");
            std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644))
                .expect("chmod");
            store.remember(a, &rose());
            let mode = std::fs::metadata(&path)
                .expect("written")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "still only its owner");
            assert_eq!(Store::at(path.clone()).lookup(a), Some(rose()));
            let _ = std::fs::remove_file(&stale);
        }
        std::fs::write(&path, "{ not json").expect("write");
        assert!(Store::at(path.clone()).entries.is_empty(), "starts over");
        // One bad entry (and an oversized one) is skipped, not the whole list.
        let long = "x".repeat(MAX_FILE + 1);
        let text = format!(
            r#"[{{"key":"k1","file":"a.fig","style":{{"name":"Kept"}}}},{{"file":7}},{{"file":"{long}","style":{{}}}}]"#
        );
        std::fs::write(&path, text).expect("write");
        let kept = Store::at(path.clone());
        assert_eq!(kept.entries.len(), 1);
        assert_eq!(kept.lookup(a).and_then(|s| s.name), Some("Kept".into()));
        // A store too big to be one is not read, and not written over either: what was
        // not read is never lost.
        let big = " ".repeat(MAX_BYTES as usize + 1);
        std::fs::write(&path, &big).expect("write");
        let mut refused = Store::at(path.clone());
        assert!(refused.entries.is_empty());
        refused.remember(a, &rose());
        assert_eq!(refused.lookup(a), Some(rose()), "kept for this session");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read").len(),
            big.len(),
            "the file is as it was"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_style_saved_before_folders_were_is_found_by_name() {
        let mut store = Store::memory();
        store.remember(id(Some("old"), "a.fig", None), &rose());
        // The same name re-exported (a new content key), now with a folder.
        let found = store.lookup(id(Some("new"), "a.fig", Some("f1")));
        assert_eq!(found, Some(rose()));
        // A row with that exact folder wins over one with none.
        let other = Style {
            name: Some("Other".into()),
            ..Style::default()
        };
        store.remember(id(Some("k2"), "a.fig", Some("f1")), &other);
        store.remember(id(Some("k3"), "a.fig", None), &rose());
        assert_eq!(
            store.lookup(id(Some("new"), "a.fig", Some("f1"))),
            Some(other)
        );
        assert_eq!(
            store.lookup(id(Some("new"), "a.fig", Some("f2"))),
            Some(rose())
        );
    }

    #[test]
    fn old_files_are_forgotten_first() {
        let mut store = Store::memory();
        for i in 0..MAX_ENTRIES + 3 {
            let (key, file) = (format!("k{i}"), format!("{i}.fig"));
            store.remember(id(Some(&key), &file, None), &rose());
        }
        assert_eq!(store.entries.len(), MAX_ENTRIES);
        assert_eq!(store.lookup(id(Some("k0"), "0.fig", None)), None);
        assert!(store.lookup(id(Some("k3"), "3.fig", None)).is_some());
    }

    #[test]
    fn every_offered_emoji_and_icon_can_be_drawn() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Cleared: epaint asserts (in debug builds) that no texture change is dropped unseen.
        ctx.run_ui(Default::default(), |_| {})
            .textures_delta
            .clear();
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
