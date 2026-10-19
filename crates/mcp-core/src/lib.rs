//! Node IDs, `figstudio://` and figma.com links, and the errors for them.
//!
//! No async runtime and no SDK types. With `--no-default-features` (no `fs`, no `system-fonts`) it builds
//! for `wasm32-unknown-unknown`; byte-backed entry readers support lazy browser images.

/// PNG, JPEG or original image bytes of nodes.
pub mod assets;
/// Browser-to-loopback bridge messages.
pub mod bridge;
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
/// The local `.fig` copy of a figma.com link, found by file name.
pub mod local_copy;
/// Sparse XML page lists and node subtrees.
pub mod metadata;
/// Accepted Figma node ID formats.
pub mod node_id;
/// Tool replies without an SDK type.
pub mod output;
/// Offline design-system prompt guidance shared by transports.
pub mod prompts;
/// The nodes a page rectangle touches.
pub mod region;
/// PNG renders of a page rectangle.
pub mod region_shot;
/// `get_nodes_in_region`: the nodes a page rectangle touches, as JSON.
pub mod region_tool;
/// Render settings and lazily read images.
pub mod rendering;
/// PNG renders of one node.
pub mod screenshot;
/// Used style and variable definitions of a subtree.
pub mod variables;
/// Browser tab snapshot and SDK-free MCP JSON-RPC dispatch.
pub mod wasm_rpc;

pub use error::CoreError;
