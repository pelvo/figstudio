//! Choosing between an eager and a lazy open by file size.
use crate::{Document, FigError};
use std::path::Path;

/// Files bigger than this open lazily: images are read when needed, not up front.
pub const LAZY_ABOVE: u64 = 256 * 1024 * 1024;

impl Document {
    /// Opens `path` eagerly, or lazily (see [`Document::open_lazy`]) when it is bigger than
    /// [`LAZY_ABOVE`].
    pub fn open_auto(path: impl AsRef<Path>) -> Result<Document, FigError> {
        Self::open_auto_above(path, LAZY_ABOVE)
    }

    /// Like [`Document::open_auto`], lazy when the file is bigger than `lazy_above` bytes.
    pub fn open_auto_above(path: impl AsRef<Path>, lazy_above: u64) -> Result<Document, FigError> {
        let path = path.as_ref();
        let big = std::fs::metadata(path).is_ok_and(|m| m.len() > lazy_above);
        if big {
            Self::open_lazy(path)
        } else {
            Self::open(path)
        }
    }
}
