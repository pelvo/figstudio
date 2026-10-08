//! Decoder for Figma `.fig` local copies (encoding back to `.fig` is planned for editing).
//!
//! A `.fig` is either a zip archive (`canvas.fig`, `meta.json`, `thumbnail.png`,
//! `images/<sha1>`) or the raw canvas. The canvas carries its own Kiwi schema, so
//! files from any Figma version decode without generated types.

mod doc;
mod val;

pub use doc::{Document, FigError, MAX_FILE_BYTES, Node};
pub use val::{Guid, Val, hex};
