//! Rendered PNG assets: stable names, files kept inside the assets directory, or inline images.
use super::jsx::Writer;
use super::walk::Target;
use super::{MAX_ASSETS, MAX_INLINE_BYTES};
use crate::tools::screenshot::{Root, layer_shot};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Where `get_design_context` puts rendered assets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Assets {
    /// PNG files written into this directory and referenced by absolute path.
    Dir(PathBuf),
    /// Base64 MCP image content items, referenced by asset name.
    Inline,
}

impl Default for Assets {
    /// `./figma-assets`, resolved against the current directory.
    fn default() -> Self {
        Self::dir(PathBuf::from("figma-assets"))
    }
}

impl Assets {
    /// A directory mode with `dir` made absolute against the current directory, so asset
    /// references stay valid wherever the client reads them from.
    pub fn dir(dir: PathBuf) -> Self {
        Self::Dir(std::path::absolute(&dir).unwrap_or(dir))
    }
}

/// A stable file name for the asset of node `id` in file `key`: only ASCII letters, digits,
/// `-` and `_` before `.png`, so it can never name a parent directory or a hidden file.
pub(super) fn name(key: &str, id: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
                ':' => '-',
                _ => '_',
            })
            .collect()
    };
    format!("{}_{}.png", clean(key), clean(id))
}

/// Writes `bytes` as `dir/name` (creating `dir`) and returns the written path. The bytes go to
/// a new temporary file (`create_new` does not follow links) that is renamed over the target,
/// so an existing link named `name` is replaced, never followed out of the directory.
pub(super) fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if Path::new(name).file_name() != Some(std::ffi::OsStr::new(name)) || name.starts_with('.') {
        return Err(std::io::Error::other(
            "asset names must be plain file names",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.{}.{unique}.tmp", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| std::fs::rename(&temp, dir.join(name)));
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map(|()| dir.join(name))
}

/// One rendered asset.
#[derive(Debug)]
pub(super) struct Asset {
    pub(super) name: String,
    pub(super) node: String,
    /// What the code's `src` holds: the written path, or the name for inline images.
    pub(super) reference: String,
    pub(super) inline: Option<Vec<u8>>,
}

/// Asset render scale: sharp on high-density screens.
const ASSET_SCALE: f64 = 2.0;

/// Renders `target` to the PNG asset `name`; `false` (counted) when it could not be
/// rendered or stored, or would exceed the asset caps.
pub(super) fn render(w: &mut Writer<'_>, target: &Target, name: String) -> bool {
    if w.out.assets.len() >= MAX_ASSETS {
        w.out.notes.add("assets not rendered (asset cap)");
        return false;
    }
    let root = match target {
        Target::Node(id) => Root::Node(*id),
        Target::Path(path) => Root::Path(path),
    };
    let Ok(shot) = layer_shot(w.doc, root, ASSET_SCALE, w.options.system_fonts) else {
        w.out.notes.add("assets not rendered (empty or hidden)");
        return false;
    };
    let approximated = !crate::tools::screenshot::approximations(&shot.frame.stats).is_empty();
    let Ok(png) = shot.frame.pixmap.into_png() else {
        w.out.notes.add("assets not rendered (empty or hidden)");
        return false;
    };
    if approximated {
        w.out
            .notes
            .add("assets with render approximations (see get_screenshot)");
    }
    let (reference, inline) = match &w.options.assets {
        Assets::Inline => {
            let encoded = png.len().div_ceil(3) * 4;
            if w.inline_bytes + encoded > MAX_INLINE_BYTES {
                w.out.notes.add("assets not rendered (inline size cap)");
                return false;
            }
            w.inline_bytes += encoded;
            (name.clone(), Some(png))
        }
        Assets::Dir(dir) => match write(dir, &name, &png) {
            Ok(path) => (path.display().to_string(), None),
            Err(_) => {
                w.out
                    .notes
                    .add("assets not written (assets directory error)");
                return false;
            }
        },
    };
    w.out.assets.push(Asset {
        name,
        node: target.id(),
        reference,
        inline,
    });
    true
}
