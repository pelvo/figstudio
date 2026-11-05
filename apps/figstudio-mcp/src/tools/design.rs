//! `get_design_context`: React + Tailwind reference code for a local node, with assets.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use figstudio_fig::Document;
use figstudio_mcp_core::design as core;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub use core::{Assets, MAX_ASSETS, MAX_CODE_BYTES, MAX_INLINE_BYTES, MAX_NODES, Options};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesignInput {
    /// Node ID, instance path (I1:2;3:4) or Figma URL with node-id. Required, except in the
    /// app's live mode, where it defaults to the selection.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// The project's frameworks, for example "React" or "Vue"; echoed in the instructions.
    client_frameworks: Option<String>,
    /// The project's languages, for example "typescript"; echoed in the instructions.
    client_languages: Option<String>,
    /// Most layers to write (default 1000, at most 5000).
    max_nodes: Option<usize>,
    /// Lossless asset format: png (default) or webp, for inline and disk images.
    format: Option<super::image_format::ImageFormat>,
}

#[tool_router(router = design_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Choose lossless asset format png (default) or webp. Get React + Tailwind reference code for a local node (data-node-id and data-name on every element), then instructions for the agent and the rendered assets. Auto-layout becomes flex, other children absolute; fills, gradients, strokes, radii, shadows, blurs, opacity, blend modes, typography and style runs are carried over; bound colour variables become var(--name,fallback). Vectors, boolean operations and image-filled leaves become <img> assets rendered locally (written to the server's assets directory, or inline images). Instances show their resolved content with overrides; nodeId may be an instance path (I1:2;3:4). Large trees are cut (maxNodes, 24 kB code cap by default (--max-output-bytes), 30 assets, 4 MB of inline images) with a note: use get_metadata for the outline, then call this on children."
    )]
    async fn get_design_context(
        &self,
        Parameters(input): Parameters<DesignInput>,
    ) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let (key, node_id) = match self.target(key, input.node_id) {
            Ok(target) => target,
            Err(error) => return tool_error(error),
        };
        let key = key.as_deref();
        let (document, file_key, path) = match self.files().open_picked(key).await {
            Ok(picked) => picked,
            Err(error) => return tool_error(error),
        };
        let rendering = match self
            .files()
            .rendering(key, &document, !self.no_system_fonts())
        {
            Ok(rendering) => rendering.with_format(input.format.unwrap_or_default().into()),
            Err(error) => return tool_error(error),
        };
        let options = Options {
            file_key,
            frameworks: input.client_frameworks,
            languages: input.client_languages,
            max_nodes: input.max_nodes,
            assets: self.assets().clone(),
            rendering,
            max_code_bytes: Some(self.max_output_bytes()),
            // Read with the file below, off the async runtime.
            code_connect: Default::default(),
        };
        match tokio::task::spawn_blocking(move || {
            let mut options = options;
            // An unreadable sidecar maps nothing here; the Code Connect tools report its error.
            options.code_connect = figstudio_mcp_core::codeconnect_store::mapped(&path)
                .unwrap_or_default()
                .into_iter()
                .map(|(id, m)| {
                    (
                        id,
                        figstudio_mcp_core::design::code_connect::CodeComponent {
                            name: m.component_name,
                            source: m.source,
                            snippet: m.snippet,
                            imports: m.snippet_imports,
                        },
                    )
                })
                .collect();
            design_context(&document, &node_id, &options)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// Code, instructions and assets for `node_id`; an unknown or unresolvable id is a tool error.
pub fn design_context(doc: &Document, node_id: &str, options: &Options) -> CallToolResult {
    to_call(core::design_context(doc, node_id, options))
}
