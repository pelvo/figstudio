//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{
    Document, Guid,
    resolve::{InstancePath, resolve_instance},
};
use figstudio_render::{
    Renderer, Stats, Viewport, absolute_transform, content_bounds,
    geom::{affine, size, transformed_bounds},
    kurbo::{Affine, Rect},
    node_bounds, page_background,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Longest output side in pixels; larger requests are scaled down to fit.
pub const MAX_SIDE_PX: f64 = 4096.0;
/// Accepted `scale` range requested by the caller (the cap may lower the one actually used).
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
        description = "Render a node to a PNG image. Pages (canvases) are drawn on the page background colour; every other node on a transparent background. scale defaults to 1 (0.01 to 4) and the longest side is capped at 4096 px; the output is cropped to the node's own box (no padding), so shadows and strokes outside it are cut off. The text reply lists what was approximated or not drawn. An instance path (I1:2;3:4) renders the whole instance cropped to that child's box, so overlapping siblings inside the instance show too."
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
        Ok(NodeId::InstancePath(path)) => {
            return match child_bounds(doc, &path) {
                Ok(bounds) => render(doc, path.instance(), bounds, scale, None, system_fonts),
                Err(message) => failure(message),
            };
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
    // `content_bounds` is a culling margin, not ink: it only tells hidden nodes apart and
    // gives a page (which has no size) its extent. Other nodes are cropped to their own box.
    let Some(content) = content_bounds(doc, id) else {
        return failure(format!("node {id} is hidden or has nothing visible"));
    };
    let bounds = if is_page {
        content
    } else {
        node_bounds(doc, id).unwrap_or(content)
    };
    let background = is_page.then(|| page_background(doc, page));
    render(doc, id, bounds, scale, background, system_fonts)
}

/// The page-space box of a node inside an instance, from the resolved transforms and size.
/// The renderer cannot draw a node inside an instance on its own (it only walks document
/// nodes), so the instance is drawn into this box instead. shortcut: uniform-scale and
/// constraint corrections the renderer applies are not repeated here, so the box can be off
/// for scaled or resized instances; share the renderer's layout step if that shows up.
fn child_bounds(doc: &Document, path: &InstancePath) -> Result<Rect, String> {
    let walk = resolve_instance(doc, path.instance()).map_err(|error| error.to_string())?;
    let chain = walk
        .ancestors(path)
        .ok_or_else(|| format!("{path} names no node inside the instance"))?;
    let leaf = chain[chain.len() - 1];
    if leaf.data.flag("visible") == Some(false) {
        return Err(format!("node {path} is hidden"));
    }
    let transform = chain
        .iter()
        .fold(absolute_transform(doc, path.instance()), |tf, node| {
            tf * affine(node.data.get("transform"))
        });
    let (w, h) = size(&leaf.data);
    if w <= 0.0 && h <= 0.0 {
        return Err(format!("node {path} has zero size, nothing to render"));
    }
    Ok(transformed_bounds(transform, Rect::new(0.0, 0.0, w, h)))
}

/// Renders `root` into `bounds` (page units) at `scale`, capped at [`MAX_SIDE_PX`].
fn render(
    doc: &Document,
    root: Guid,
    bounds: Rect,
    scale: f64,
    background: Option<figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>>,
    system_fonts: bool,
) -> CallToolResult {
    let longest = bounds.width().max(bounds.height()) * scale;
    if !longest.is_finite() || longest <= 0.0 {
        return failure(format!("node {root} has no finite size to render"));
    }
    // Cap the longest side by lowering the scale (no floor: a huge node needs a tiny scale);
    // the epsilon keeps `ceil` from rounding the side up to 4097.
    let scale = if longest > MAX_SIDE_PX {
        scale * MAX_SIDE_PX / longest * (1.0 - 1e-9)
    } else {
        scale
    };
    // The epsilon absorbs float noise so a 381-unit box at scale 1 is 381 px, not 382.
    let pixels =
        |extent: f64| ((extent * scale - 1e-6).ceil().max(1.0) as u16).min(MAX_SIDE_PX as u16);
    let (width, height) = (pixels(bounds.width()), pixels(bounds.height()));
    let viewport = Viewport {
        width,
        height,
        transform: Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0)),
        background,
    };
    let mut renderer = Renderer::new();
    renderer.system_fonts = system_fonts;
    let frame = renderer.render(doc, root, &viewport);
    let png = match frame.pixmap.into_png() {
        Ok(png) => png,
        Err(error) => return failure(format!("PNG encoding failed: {error}")),
    };
    let mut text = format!("{width}x{height} px, scale {}", significant(scale));
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

/// `value` rounded to four significant figures, so a tiny capped scale is not shown as 0.
fn significant(value: f64) -> f64 {
    let digits = 3 - value.log10().floor() as i32;
    let factor = 10f64.powi(digits);
    (value * factor).round() / factor
}

/// The non-zero `Stats` counters that mean something was drawn inexactly or not at all,
/// labelled as in `Stats`' own display. `culled` counts layers left out because they fall
/// outside the node's box or are smaller than a pixel.
fn approximations(stats: &Stats) -> Vec<String> {
    [
        (stats.text_missing, "text not drawn"),
        (stats.text_truncation_ignored, "text truncation ignored"),
        (stats.approximated_effects, "shadow effects approximated"),
        (stats.skipped_effects, "effects skipped"),
        (stats.missing_images, "images missing"),
        (stats.images_loading, "images loading"),
        (stats.missing_symbols, "components missing"),
        (stats.unsupported_paints, "paints skipped"),
        (stats.approximated_blends, "blend modes approximated"),
        (stats.approximated_paints, "paints approximated"),
        (stats.approximated_strokes, "stroke alignments approximated"),
        (stats.approximated_instance_ink, "instance ink approximated"),
        (stats.culled, "off screen or subpixel layers omitted"),
        (stats.budget_exhausted, "layers skipped (work budget)"),
        (stats.depth_skipped, "layers skipped (depth)"),
        (stats.preparation_skipped, "bounds omitted (budget)"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, what)| format!("{count} {what}"))
    .collect()
}
