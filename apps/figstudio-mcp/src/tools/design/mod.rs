//! `get_design_context`: React + Tailwind reference code for a local node, with assets.
mod assets;
mod jsx;
mod layout;
mod paint;
mod style;
mod text;
mod walk;

use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
pub use assets::Assets;
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::collections::BTreeMap;
use walk::Target;

/// Default and largest `maxNodes`.
pub const MAX_NODES: (usize, usize) = (1_000, 5_000);
/// Code beyond this many bytes is cut at a layer boundary, with a note.
pub const MAX_CODE_BYTES: usize = 100_000;
/// Rendered assets per call; further vectors and images stay placeholders.
pub const MAX_ASSETS: usize = 30;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesignInput {
    /// Node ID, instance path (I1:2;3:4) or Figma URL with node-id.
    node_id: String,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// The project's frameworks, for example "React" or "Vue"; echoed in the instructions.
    client_frameworks: Option<String>,
    /// The project's languages, for example "typescript"; echoed in the instructions.
    client_languages: Option<String>,
    /// Most layers to write (default 1000, at most 5000).
    max_nodes: Option<usize>,
}

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
    /// Whether asset renders may use installed fonts.
    pub system_fonts: bool,
}

#[tool_router(router = design_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get React + Tailwind reference code for a local node (data-node-id and data-name on every element), then instructions for the agent and the rendered assets. Auto-layout becomes flex, other children absolute; fills, gradients, strokes, radii, shadows, blurs, opacity, blend modes, typography and style runs are carried over; bound colour variables become var(--name,fallback). Vectors, boolean operations and image-filled leaves become <img> PNG assets rendered locally (written to the server's assets directory, or inline images). Instances show their resolved content with overrides; nodeId may be an instance path (I1:2;3:4). Large trees are cut (maxNodes, size cap) with a note: use get_metadata for the outline, then call this on children."
    )]
    async fn get_design_context(
        &self,
        Parameters(input): Parameters<DesignInput>,
    ) -> CallToolResult {
        let key = input.file_key.as_deref();
        let (document, file_key) = match (self.files().open(key).await, self.files().key(key)) {
            (Ok(document), Ok(file_key)) => (document, file_key),
            (Err(error), _) | (_, Err(error)) => return tool_error(error),
        };
        let options = Options {
            file_key,
            frameworks: input.client_frameworks,
            languages: input.client_languages,
            max_nodes: input.max_nodes,
            assets: self.assets().clone(),
            system_fonts: !self.no_system_fonts(),
        };
        match tokio::task::spawn_blocking(move || {
            design_context(&document, &input.node_id, &options)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

fn failure(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

/// Code, instructions and assets for `node_id`; an unknown or unresolvable id is a tool error.
pub fn design_context(doc: &Document, node_id: &str, options: &Options) -> CallToolResult {
    let root = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) if doc.node(id).is_some_and(|n| n.kind != "DOCUMENT") => {
            Target::Node(id)
        }
        Ok(NodeId::Node(id)) => {
            return failure(format!(
                "Unknown node ID {id}; use get_metadata to list pages and layers"
            ));
        }
        Ok(NodeId::InstancePath(path)) => Target::Path(path),
        Err(error) => return tool_error(error),
    };
    let limit = options
        .max_nodes
        .unwrap_or(MAX_NODES.0)
        .clamp(1, MAX_NODES.1);
    let Some(layers) = walk::collect(doc, &root, limit) else {
        return failure(format!(
            "Unknown node ID {}; use get_metadata to list pages and layers",
            root.id()
        ));
    };
    let out = jsx::write(doc, &layers, options);
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
                images.push(ContentBlock::image(STANDARD.encode(png), "image/png"));
            }
        }
    }
    if let Some(line) = notes.line() {
        text.push('\n');
        text.push_str(&line);
        text.push('\n');
    }
    let mut content = vec![ContentBlock::text(out.code), ContentBlock::text(text)];
    content.extend(images);
    CallToolResult::success(content)
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
