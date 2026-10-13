//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    node_id::{NodeId, parse_node_id},
    output::{Block, Output},
    rendering::Rendering,
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

/// Longest output side in pixels when the caller gives an explicit `scale`; larger requests are
/// scaled down to fit. The hard cap: 4096 px PNGs were 3.1-3.7 MB, near the 5 MB base64 limit.
pub const MAX_SIDE_PX: f64 = 4096.0;
/// Longest side when the caller omits `scale`. Claude downscales images above 1568 px, so 2048
/// px keeps the default render within the 5 MB base64 limit with room to spare.
pub const DEFAULT_SIDE_PX: f64 = 2048.0;
/// Accepted `scale` range requested by the caller (the cap may lower the one actually used).
pub const SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.01..=4.0;

/// Render `node_id` to a base64 PNG plus a size and approximations line.
pub fn screenshot(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    rendering: &Rendering,
) -> Output {
    let shot = match render_target(doc, node_id, scale, rendering) {
        Ok(shot) => shot,
        Err(message) => return Output::fault(message),
    };
    let text = shot.describe();
    match shot.frame.pixmap.into_png() {
        Ok(png) => Output::success(vec![
            Block::image(STANDARD.encode(png), "image/png"),
            Block::text(text),
        ]),
        Err(error) => Output::fault(format!("PNG encoding failed: {error}")),
    }
}

/// A region link (`figstudio://<key>?page=…&rect=…`) renders that part of the page; anything
/// else is a node, as in [`render_node`].
fn render_target(
    doc: &Document,
    target: &str,
    scale: Option<f64>,
    rendering: &Rendering,
) -> Result<Shot, String> {
    if target.starts_with("figstudio://")
        && let Ok(link) = crate::link::parse_link(target)
        && let (Some(page), Some(rect)) = (link.page, link.rect)
    {
        return crate::region_shot::region_shot(doc, page, rect, scale, rendering);
    }
    render_node(doc, target, scale, rendering)
}

/// A finished render of one node: pixels plus what the renderer reported.
pub struct Shot {
    pub frame: Frame,
    pub scale: f64,
}

impl Shot {
    /// "WxH px, scale S", then an approximations line when anything was drawn inexactly.
    pub fn describe(&self) -> String {
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
pub fn render_node(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    rendering: &Rendering,
) -> Result<Shot, String> {
    // An omitted scale gets the smaller side cap; an explicit one keeps the 4096 px cap.
    let max_side = if scale.is_some() {
        MAX_SIDE_PX
    } else {
        DEFAULT_SIDE_PX
    };
    let scale = scale.unwrap_or(1.0);
    if !SCALE_RANGE.contains(&scale) {
        return Err("scale must be between 0.01 and 4".into());
    }
    let id = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) => id,
        Ok(NodeId::InstancePath(path)) => {
            return instance_child(doc, &path, scale, max_side, rendering);
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
    render(
        doc,
        Root::Node(id),
        bounds,
        scale,
        background,
        max_side,
        rendering,
    )
}

/// What a screenshot draws: a document node, or one node inside an instance on its own.
#[derive(Clone, Copy)]
pub enum Root<'p> {
    Node(Guid),
    Path(&'p InstancePath),
}

/// Renders the instance child at `path` alone, cropped to the box the renderer gives it.
fn instance_child(
    doc: &Document,
    path: &InstancePath,
    scale: f64,
    max_side: f64,
    rendering: &Rendering,
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
    render(
        doc,
        Root::Path(path),
        bounds,
        scale,
        None,
        max_side,
        rendering,
    )
}

/// Renders a layer (not a page) cropped to its own box on a transparent background, as
/// `get_screenshot` does. Used by `get_design_context` assets.
pub fn layer_shot(
    doc: &Document,
    root: Root<'_>,
    scale: f64,
    rendering: &Rendering,
) -> Result<Shot, String> {
    let bounds = match root {
        Root::Node(id) => node_bounds(doc, id),
        Root::Path(path) => Renderer::new().path_bounds(doc, path),
    }
    .ok_or("hidden or nothing visible")?;
    render(doc, root, bounds, scale, None, MAX_SIDE_PX, rendering)
}

/// Renders `root` into `bounds` (page units) at `scale`, its longest side capped at `max_side`.
fn render(
    doc: &Document,
    root: Root<'_>,
    bounds: Rect,
    scale: f64,
    background: Option<figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>>,
    max_side: f64,
    rendering: &Rendering,
) -> Result<Shot, String> {
    let longest = bounds.width().max(bounds.height()) * scale;
    if !longest.is_finite() || longest <= 0.0 {
        return Err("the node has no finite size to render".into());
    }
    // Cap the longest side by lowering the scale (no floor: a huge node needs a tiny scale);
    // the epsilon keeps `ceil` from rounding the side up to 4097.
    let scale = if longest > max_side {
        scale * max_side / longest * (1.0 - 1e-9)
    } else {
        scale
    };
    // The epsilon absorbs float noise so a 381-unit box at scale 1 is 381 px, not 382.
    let pixels =
        |extent: f64| ((extent * scale - 1e-6).ceil().max(1.0) as u16).min(max_side as u16);
    let (width, height) = (pixels(bounds.width()), pixels(bounds.height()));
    let viewport = Viewport {
        width,
        height,
        transform: Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0)),
        background,
    };
    let frame = rendering.draw(doc, root, &viewport);
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
pub fn approximations(stats: &Stats) -> Vec<String> {
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
