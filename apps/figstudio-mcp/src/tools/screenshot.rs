//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::Document;
use figstudio_render::{Renderer, content_bounds, page_background};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Longest output side in pixels; larger requests are scaled down to fit.
pub const MAX_SIDE_PX: f64 = 4096.0;
/// Accepted `scale` range (the renderer clamps below 0.01).
pub const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.01..=4.0;

/// Inputs for a node screenshot.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotInput {
    /// Node ID or Figma URL with `node-id`.
    node_id: String,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Pixels per design unit, 0.01 to 4 (default 1). The longest side is capped at 4096 px.
    scale: Option<f64>,
}

#[tool_router(router = screenshot_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Render a node to a PNG image. Pages (canvases) are drawn on the page background colour; every other node on a transparent background. scale defaults to 1 (0.01 to 4) and the longest side is capped at 4096 px; the output has about 1 design unit of margin for shadows and strokes. The text reply lists what was approximated or not drawn. Instance paths are not supported yet."
    )]
    async fn get_screenshot(
        &self,
        Parameters(input): Parameters<ScreenshotInput>,
    ) -> CallToolResult {
        let document = match self.files().open(input.file_key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let system_fonts = !self.no_system_fonts();
        match tokio::task::spawn_blocking(move || {
            screenshot(&document, &input.node_id, input.scale, system_fonts)
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

/// Render `node_id` to a base64 PNG plus a size and approximations line.
pub fn screenshot(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    system_fonts: bool,
) -> CallToolResult {
    let scale = scale.unwrap_or(1.0);
    if !SCALE_RANGE.contains(&scale) {
        return failure("scale must be between 0.01 and 4");
    }
    let id = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) => id,
        Ok(NodeId::InstancePath(_)) => {
            return failure("instance paths are not yet supported for screenshots");
        }
        Err(error) => return tool_error(error),
    };
    let Some(node) = doc.node(id) else {
        return failure(format!("node {id} does not exist in this file"));
    };
    if node.kind == "DOCUMENT" {
        return failure("pick a page or a layer, not the document");
    }
    let Some(page) = doc.page_of(id) else {
        return failure(format!("node {id} is not on a page"));
    };
    let is_page = node.kind == "CANVAS";
    let size = |axis: &str| node.data.get("size").and_then(|s| s.num(axis));
    if !is_page && size("x").unwrap_or(0.0) <= 0.0 && size("y").unwrap_or(0.0) <= 0.0 {
        return failure(format!("node {id} has zero size, nothing to render"));
    }
    let Some(bounds) = content_bounds(doc, id) else {
        return failure(format!("node {id} is hidden or has nothing visible"));
    };
    // Cap the longest side; the epsilon keeps `ceil` inside the renderer from rounding up.
    let longest = bounds.width().max(bounds.height()) * scale;
    let scale = if longest > MAX_SIDE_PX {
        scale * MAX_SIDE_PX / longest * (1.0 - 1e-9)
    } else {
        scale
    };
    let background = is_page.then(|| page_background(doc, page));
    let mut renderer = Renderer::new();
    renderer.system_fonts = system_fonts;
    let Some(frame) = renderer.render_fit(doc, id, scale, 0.0, background) else {
        return failure(format!("node {id} is hidden or has nothing visible"));
    };
    let (width, height) = (frame.pixmap.width(), frame.pixmap.height());
    let png = match frame.pixmap.into_png() {
        Ok(png) => png,
        Err(error) => return failure(format!("PNG encoding failed: {error}")),
    };
    let scale = (scale * 1000.0).round() / 1000.0;
    let mut text = format!("{width}x{height} px, scale {scale}");
    let notes = approximations(&frame.stats);
    if !notes.is_empty() {
        text.push_str("\napproximations: ");
        text.push_str(&notes.join(", "));
    }
    CallToolResult::success(vec![
        ContentBlock::image(STANDARD.encode(png), "image/png"),
        ContentBlock::text(text),
    ])
}

/// The non-zero `Stats` counters that mean something was drawn inexactly or not at all.
fn approximations(stats: &figstudio_render::Stats) -> Vec<String> {
    const GAPS: [&str; 7] = [
        "skipped",
        "missing",
        "approximated",
        "not drawn",
        "ignored",
        "loading",
        "omitted",
    ];
    stats
        .to_string()
        .split(", ")
        .filter(|part| GAPS.iter().any(|gap| part.contains(gap)))
        .map(str::to_owned)
        .collect()
}
