//! Node IDs, `figstudio://` and figma.com links, and the errors for them.
//!
//! No async runtime and no SDK types. With `--no-default-features` (no `fs`, no `system-fonts`) it builds
//! for `wasm32-unknown-unknown`; lazy image reads use `std::fs`, which fails at run time there.

/// PNG, JPEG or original image bytes of nodes.
pub mod assets;
/// Local Code Connect component context.
pub mod codeconnect_context;
/// React + Tailwind design context with rendered assets.
pub mod design;
/// Local component, style and variable search.
pub mod design_system;
/// Errors for node IDs, file keys, links and loading a document.
pub mod error;
/// FigJam boards as metadata XML.
pub mod figjam;
/// Writing files (needs the `fs` feature).
#[cfg(feature = "fs")]
pub mod fs;
/// `figstudio://` and figma.com links.
pub mod link;
/// Sparse XML page lists and node subtrees.
pub mod metadata;
/// Accepted Figma node ID formats.
pub mod node_id;
/// Tool replies without an SDK type.
pub mod output;
/// The nodes a page rectangle touches.
pub mod region;
/// Render settings and lazily read images.
pub mod rendering;
/// PNG renders of one node.
pub mod screenshot;
/// Used style and variable definitions of a subtree.
pub mod variables;

pub use error::CoreError;
