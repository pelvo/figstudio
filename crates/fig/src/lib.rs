//! Reader and writer for Figma `.fig` local copies.
//!
//! A `.fig` is either a zip archive (`canvas.fig`, `meta.json`, `thumbnail.png`,
//! `images/<sha1>`) or the raw canvas. The canvas carries its own Kiwi schema, so
//! files from any Figma version decode without generated types.

pub mod archive;
mod doc;
mod read;
mod val;
mod write;

pub use doc::{Codec, Document, FigError, MAX_FILE_BYTES, Node};
pub use val::{Guid, Obj, Val, hex, intern};
