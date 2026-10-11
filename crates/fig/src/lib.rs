//! Reader and writer for Figma `.fig` local copies.
//!
//! A `.fig` is either a zip archive (`canvas.fig`, `meta.json`, `thumbnail.png`,
//! `images/<sha1>`) or the raw canvas. The canvas carries its own Kiwi schema, so
//! files from any Figma version decode without generated types.

pub mod archive;
mod doc;
mod open;
mod read;
pub mod resolve;
pub mod tokens;
mod val;
mod write;

pub use doc::{
    Codec, Document, FigError, MAX_ARCHIVE_INFLATED_BYTES, MAX_CHUNK_BYTES, MAX_FILE_BYTES, Node,
    read_limited_file,
};
pub use open::LAZY_ABOVE;
pub use val::{Guid, Obj, Val, hex, intern};
