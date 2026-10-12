//! Node IDs, `figstudio://` and figma.com links, and the errors for them.
//!
//! No async runtime and no I/O: this crate builds for `wasm32-unknown-unknown`.

/// Errors for node IDs, file keys, links and loading a document.
pub mod error;
/// `figstudio://` and figma.com links.
pub mod link;
/// Accepted Figma node ID formats.
pub mod node_id;
/// Tool replies without an SDK type.
pub mod output;
/// The nodes a page rectangle touches.
pub mod region;
/// Used style and variable definitions of a subtree.
pub mod variables;

pub use error::CoreError;
