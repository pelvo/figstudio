//! `get_figjam`: `get_metadata` XML plus FigJam details, with optional small screenshots.
use crate::{
    FigstudioServer,
    tools::{
        assets::Payload,
        metadata::{figjam::Extras, run},
        screenshot::render_node,
        tool_error,
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Most screenshots one call returns.
pub const MAX_IMAGES: usize = 20;
/// Largest `scale` for those screenshots; they are previews, not exports.
pub const MAX_IMAGE_SCALE: f64 = 0.5;
const DEFAULT_IMAGE_SCALE: f64 = 0.25;

/// Inputs for a FigJam board listing.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FigjamInput {
    /// Node ID or Figma URL; omit for the page list.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Maximum child depth, with the requested root at depth zero.
    depth: Option<u32>,
    /// Also return a small PNG of each sticky, shape, section, table, code block, widget and
    /// stamp (at most 20, in document order). Default false.
    include_images: Option<bool>,
    /// Pixels per design unit for those images, 0.01 to 0.5 (default 0.25).
    image_scale: Option<f64>,
}

#[tool_router(router = figjam_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get a FigJam board as sparse XML (same shape as get_metadata) with FigJam details: sticky text and colour, shape-with-text shape and text, connector start/end node ids and magnets, section status, table cell and code block text, widget and stamp names. Text caps at 200 characters. includeImages=true adds a small PNG per sticky/shape/section/table/code block/widget/stamp (max 20, imageScale up to 0.5). FigJam kinds are not drawn by the renderer yet: images show them as plain boxes, and an XML comment counts those kinds."
    )]
    async fn get_figjam(&self, Parameters(input): Parameters<FigjamInput>) -> CallToolResult {
        let document = match self.files().open(input.file_key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let system_fonts = !self.no_system_fonts();
        match tokio::task::spawn_blocking(move || {
            figjam(
                &document,
                input.node_id.as_deref(),
                input.depth,
                input.include_images.unwrap_or(false),
                input.image_scale,
                system_fonts,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// The FigJam XML for `node_id`, then one text line and PNG per pictured node when asked.
pub fn figjam(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    system_fonts: bool,
) -> CallToolResult {
    figjam_within(
        doc,
        node_id,
        depth,
        include_images,
        image_scale,
        system_fonts,
        crate::tools::assets::MAX_PAYLOAD_BYTES,
    )
}

/// [`figjam`] with an explicit image payload budget in bytes (before base64).
pub fn figjam_within(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    system_fonts: bool,
    max_payload: usize,
) -> CallToolResult {
    let scale = image_scale.unwrap_or(DEFAULT_IMAGE_SCALE);
    if !(0.01..=MAX_IMAGE_SCALE).contains(&scale) {
        return CallToolResult::error(vec![ContentBlock::text(
            "imageScale must be between 0.01 and 0.5",
        )]);
    }
    let (xml, extras) = match run(doc, node_id, depth, Some(Extras::default())) {
        Ok((xml, extras)) => (xml, extras.unwrap_or_default()),
        Err(result) => return result,
    };
    let mut content = vec![ContentBlock::text(xml)];
    if include_images {
        let mut payload = Payload::new(max_payload);
        for id in extras.pictured.iter().take(MAX_IMAGES) {
            content.extend(picture(doc, id, scale, system_fonts, &mut payload));
        }
        let skipped = extras.pictured.len().saturating_sub(MAX_IMAGES);
        if skipped > 0 {
            content.push(ContentBlock::text(format!(
                "{skipped} more node(s) not pictured (limit {MAX_IMAGES})"
            )));
        }
    }
    CallToolResult::success(content)
}

/// A text line and PNG for one node, or only a line when it cannot be rendered or no longer
/// fits the reply's image budget.
fn picture(
    doc: &Document,
    id: &str,
    scale: f64,
    system_fonts: bool,
    payload: &mut Payload,
) -> Vec<ContentBlock> {
    if payload.is_full() {
        return vec![ContentBlock::text(Payload::omitted(id))];
    }
    let shot = match render_node(doc, id, Some(scale), system_fonts) {
        Ok(shot) => shot,
        Err(message) => return vec![ContentBlock::text(format!("{id}: no image: {message}"))],
    };
    let line = format!("{id}: {}", shot.describe());
    match shot.frame.pixmap.into_png() {
        Ok(png) if payload.take(png.len()) => vec![
            ContentBlock::text(line),
            ContentBlock::image(STANDARD.encode(png), "image/png"),
        ],
        Ok(_) => vec![ContentBlock::text(Payload::omitted(id))],
        Err(error) => vec![ContentBlock::text(format!(
            "{id}: PNG encoding failed: {error}"
        ))],
    }
}
