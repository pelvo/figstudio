//! `get_design_context`: React + Tailwind reference code for a local node, with assets.
mod assets;
mod jsx;
mod layout;
mod paint;
mod style;
mod text;
mod walk;

use crate::{
    node_id::{NodeId, parse_node_id},
    output::{Block, Output},
    rendering::Rendering,
};
pub use assets::Assets;
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use std::collections::BTreeMap;
use walk::Target;

/// Default and largest `maxNodes`.
pub const MAX_NODES: (usize, usize) = (1_000, 5_000);
/// Default code bytes before a cut at a layer boundary, with a note. Claude Code's inline
/// limit is about 25k tokens: 25,810 chars were delivered, 44,674 were rejected. This stays
/// under the delivered size; `--max-output-bytes` overrides it.
pub const MAX_CODE_BYTES: usize = 24_000;
/// Rendered assets per call; further vectors and images stay placeholders.
pub const MAX_ASSETS: usize = 30;
/// Base64 bytes of inline images per reply; further assets stay placeholders.
pub const MAX_INLINE_BYTES: usize = 4_000_000;

/// Inputs of [`design_context`] besides the document and node.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Registered file key; prefixes asset names.
    pub file_key: String,
    /// Echoed project frameworks.
    pub frameworks: Option<String>,
    /// Echoed project languages.
    pub languages: Option<String>,
    /// Layer cap; `None` is [`MAX_NODES`]`.0`.
    pub max_nodes: Option<usize>,
    /// Where rendered assets go.
    pub assets: Assets,
    /// How assets are rendered: fonts, lazily opened images.
    pub rendering: Rendering,
    /// Code byte cap; `None` is [`MAX_CODE_BYTES`].
    pub max_code_bytes: Option<usize>,
}

/// Code, instructions and assets for `node_id`; an unknown or unresolvable id is a tool error.
pub fn design_context(doc: &Document, node_id: &str, options: &Options) -> Output {
    #[cfg(not(feature = "fs"))]
    if matches!(options.assets, Assets::Dir(_)) {
        return Output::fault("asset files are not available in this build");
    }
    let root = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) if doc.node(id).is_some_and(|n| n.kind != "DOCUMENT") => {
            Target::Node(id)
        }
        Ok(NodeId::Node(id)) => {
            return Output::fault(format!(
                "Unknown node ID {id}; use get_metadata to list pages and layers"
            ));
        }
        Ok(NodeId::InstancePath(path)) => Target::Path(path),
        Err(error) => return Output::fault(error.to_string()),
    };
    let limit = options
        .max_nodes
        .unwrap_or(MAX_NODES.0)
        .clamp(1, MAX_NODES.1);
    let Some(layers) = walk::collect(doc, &root, limit) else {
        return Output::fault(format!(
            "Unknown node ID {}; use get_metadata to list pages and layers",
            root.id()
        ));
    };
    let out = jsx::write(
        doc,
        &layers,
        options,
        options.max_code_bytes.unwrap_or(MAX_CODE_BYTES),
    );
    let mut notes = out.notes;
    if layers.skipped_instances > 0 {
        notes.count("instances not expanded", layers.skipped_instances);
    }
    let mut text = String::from("## Instructions for the agent\n");
    text.push_str(&instructions(options));
    if layers.truncated || out.truncated {
        text.push_str("\nThe code above is TRUNCATED (layer or size cap). Call get_metadata on this node for the outline, then get_design_context on the children you need.\n");
    }
    let mut images = Vec::new();
    if !out.assets.is_empty() {
        text.push_str("\n## Assets\n");
        for asset in &out.assets {
            text.push_str(&format!(
                "- {} (node {}): {}\n",
                asset.name, asset.node, asset.reference
            ));
            if let Some(png) = &asset.inline {
                images.push(Block::image(
                    STANDARD.encode(png),
                    options.rendering.format.mime(),
                ));
            }
        }
    }
    if let Some(line) = notes.line() {
        text.push('\n');
        text.push_str(&line);
        text.push('\n');
    }
    let mut content = vec![Block::text(out.code), Block::text(text)];
    content.extend(images);
    Output::success(content)
}

fn instructions(options: &Options) -> String {
    let target = match (&options.frameworks, &options.languages) {
        (None, None) => "the project's framework and language".to_owned(),
        (f, l) => [f.as_deref(), l.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" / "),
    };
    format!(
        "- This React + Tailwind code is a reference for the design, not final code. Rewrite it for {target}, reusing the project's existing components, layout primitives and design tokens instead of copying the arbitrary values.\n\
         - var(--name,fallback) marks a bound design variable: map it to the project's token of that name.\n\
         - Use the listed assets as they are (do not draw placeholders); each replaces the layer with the same data-node-id.\n\
         - Check the result visually against get_screenshot for this node.\n\
         - For large trees use get_metadata for the outline and call get_design_context on the children.\n"
    )
}

/// Counted approximations, in label order.
#[derive(Debug, Default)]
pub(super) struct Notes(BTreeMap<&'static str, usize>);

impl Notes {
    pub(super) fn add(&mut self, what: &'static str) {
        self.count(what, 1);
    }
    pub(super) fn count(&mut self, what: &'static str, n: usize) {
        *self.0.entry(what).or_default() += n;
    }
    fn line(&self) -> Option<String> {
        (!self.0.is_empty()).then(|| {
            let parts: Vec<String> = self
                .0
                .iter()
                .map(|(what, n)| format!("{n} {what}"))
                .collect();
            format!("approximations: {}", parts.join(", "))
        })
    }
}

/// Tailwind classes of one element.
#[derive(Debug, Default)]
pub(super) struct Classes(Vec<String>);

impl Classes {
    pub(super) fn push(&mut self, class: impl Into<String>) {
        self.0.push(class.into());
    }
    /// `relative` makes the element the containing block of absolute children; an
    /// `absolute` element already is one.
    pub(super) fn relative(&mut self) {
        if !self.0.iter().any(|c| c.starts_with("absolute")) {
            self.push("relative");
        }
    }
    pub(super) fn join(&self) -> String {
        self.0.join(" ")
    }
}

/// A number with at most two decimals and no negative zero.
pub(super) fn num(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if !rounded.is_finite() || rounded == 0.0 {
        "0".into()
    } else {
        rounded.to_string()
    }
}

/// `num` in px.
pub(super) fn px(value: f64) -> String {
    format!("{}px", num(value))
}
