//! Reader and writer for Figma `.fig` local copies.
//!
//! A `.fig` is either a zip archive (`canvas.fig`, `meta.json`, `thumbnail.png`,
//! `images/<sha1>`) or the raw canvas. The canvas carries its own Kiwi schema, so
//! files from any Figma version decode without generated types.

pub mod archive;
pub mod census;
pub mod clipboard;
mod doc;
pub mod edit;
mod error;
mod open;
mod read;
pub mod resolve;
pub mod sha1;
pub mod tokens;
mod val;
mod val_serde;
mod write;

pub use doc::{
    Codec, Document, FigError, MAX_ARCHIVE_INFLATED_BYTES, MAX_CHUNK_BYTES, MAX_FILE_BYTES, Node,
    read_limited_file,
};
pub use error::one_line;
pub use open::LAZY_ABOVE;
pub use val::{Guid, Lazy, Obj, Val, hex, intern};
