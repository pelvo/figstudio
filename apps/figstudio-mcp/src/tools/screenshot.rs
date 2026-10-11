//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{
    Document, Guid,
    resolve::{InstancePath, resolve_path},
};
use figstudio_render::{
    Frame, Renderer, Stats, Viewport, content_bounds,
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
        description = "Render a node to a PNG image. Pages (canvases) are drawn on the page background colour; every other node on a transparent background. scale defaults to 1 (0.01 to 4) and the longest side is capped at 4096 px; the output is cropped to the node's own box (no padding), so shadows and strokes outside it are cut off. The text reply lists what was approximated or not drawn. An instance path (I1:2;3:4) renders that node alone (no siblings, no ancestor clips) as laid out inside its instance."
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
    let shot = match render_node(doc, node_id, scale, system_fonts) {
        Ok(shot) => shot,
        Err(message) => return failure(message),
    };
    let text = shot.describe();
    match shot.frame.pixmap.into_png() {
        Ok(png) => CallToolResult::success(vec![
            ContentBlock::image(STANDARD.encode(png), "image/png"),
            ContentBlock::text(text),
        ]),
        Err(error) => failure(format!("PNG encoding failed: {error}")),
    }
}

/// A finished render of one node: pixels plus what the renderer reported.
pub(crate) struct Shot {
    pub(crate) frame: Frame,
    pub(crate) scale: f64,
}

impl Shot {
    /// "WxH px, scale S", then an approximations line when anything was drawn inexactly.
    pub(crate) fn describe(&self) -> String {
        let mut text = format!(
            "{}x{} px, scale {}",
            self.frame.pixmap.width(),
            self.frame.pixmap.height(),
            significant(self.scale)
        );
        let notes = approximations(&self.frame.stats);
        if !notes.is_empty() {
            text.push_str("\napproximations: ");
            text.push_str(&notes.join(", "));
        }
        text
    }
}

/// Renders any accepted node id (document node or instance path) the way `get_screenshot`
/// does; `Err` is the message for a tool error. Shared by `download_assets` and `get_figjam`.
pub(crate) fn render_node(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    system_fonts: bool,
) -> Result<Shot, String> {
    let scale = scale.unwrap_or(1.0);
    if !SCALE_RANGE.contains(&scale) {
        return Err("scale must be between 0.01 and 4".into());
    }
    let id = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) => id,
        Ok(NodeId::InstancePath(path)) => {
            return instance_child(doc, &path, scale, system_fonts);
        }
        Err(error) => return Err(error.to_string()),
    };
    let Some(node) = doc.node(id) else {
        return Err(format!("node {id} does not exist in this file"));
    };
    if node.kind == "DOCUMENT" {
        return Err("pick a page or a layer, not the document".into());
    }
    let Some(page) = doc.page_of(id) else {
        return Err(format!("node {id} is not on a page"));
    };
    let is_page = node.kind == "CANVAS";
    let size = |axis: &str| node.data.get("size").and_then(|s| s.num(axis));
    if !is_page && size("x").unwrap_or(0.0) <= 0.0 && size("y").unwrap_or(0.0) <= 0.0 {
        return Err(format!("node {id} has zero size, nothing to render"));
    }
    // `content_bounds` is a culling margin, not ink: it only tells hidden nodes apart and
    // gives a page (which has no size) its extent. Other nodes are cropped to their own box.
    let Some(content) = content_bounds(doc, id) else {
        return Err(format!("node {id} is hidden or has nothing visible"));
    };
    let bounds = if is_page {
        content
    } else {
        node_bounds(doc, id).unwrap_or(content)
    };
    let background = is_page.then(|| page_background(doc, page));
    render(doc, Root::Node(id), bounds, scale, background, system_fonts)
}

/// What a screenshot draws: a document node, or one node inside an instance on its own.
#[derive(Clone, Copy)]
enum Root<'p> {
    Node(Guid),
    Path(&'p InstancePath),
}

/// Renders the instance child at `path` alone, cropped to the box the renderer gives it.
fn instance_child(
    doc: &Document,
    path: &InstancePath,
    scale: f64,
    system_fonts: bool,
) -> Result<Shot, String> {
    if let Err(error) = resolve_path(doc, path) {
        return Err(format!("{path}: {error}"));
    }
    let Some(bounds) = Renderer::new().path_bounds(doc, path) else {
        return Err(format!("node {path} is hidden or has nothing visible"));
    };
    if bounds.width() <= 0.0 && bounds.height() <= 0.0 {
        return Err(format!("node {path} has zero size, nothing to render"));
    }
    render(doc, Root::Path(path), bounds, scale, None, system_fonts)
}

/// Renders `root` into `bounds` (page units) at `scale`, capped at [`MAX_SIDE_PX`].
fn render(
    doc: &Document,
    root: Root<'_>,
    bounds: Rect,
    scale: f64,
    background: Option<figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>>,
    system_fonts: bool,
) -> Result<Shot, String> {
    let longest = bounds.width().max(bounds.height()) * scale;
    if !longest.is_finite() || longest <= 0.0 {
        return Err("the node has no finite size to render".into());
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
    let frame = match root {
        Root::Node(id) => renderer.render(doc, id, &viewport),
        Root::Path(path) => renderer.render_path(doc, path, &viewport),
    };
    Ok(Shot { frame, scale })
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
