//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    error::CoreError,
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
        Err(error) => return Output::fault(error.to_string()),
    };
    let text = shot.describe();
    match figstudio_render::encoding::encode_pixmap(shot.frame.pixmap, rendering.format) {
        Ok(png) => Output::success(vec![
            Block::image(STANDARD.encode(png), rendering.format.mime()),
            Block::text(text),
        ]),
        Err(error) => {
            Output::fault(CoreError::encode(rendering.format.extension(), error).to_string())
        }
    }
}

/// A region link (`figstudio://<key>?page=…&rect=…`) renders that part of the page; anything
/// else is a node, as in [`render_node`].
fn render_target(
    doc: &Document,
    target: &str,
    scale: Option<f64>,
    rendering: &Rendering,
) -> Result<Shot, CoreError> {
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
) -> Result<Shot, CoreError> {
    with_target(
        doc,
        node_id,
        scale,
        |root, bounds, scale, background, max_side| {
            render(doc, root, bounds, scale, background, max_side, rendering)
        },
    )
}

/// A finished SVG export of one node, cropped like [`render_node`].
pub struct SvgShot {
    pub svg: figstudio_render::svg::Svg,
    pub width: u16,
    pub height: u16,
    pub scale: f64,
}

impl SvgShot {
    /// "WxH, scale S, N bytes of SVG", then an approximations line when anything was
    /// drawn inexactly.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{}x{}, scale {}, {} bytes of SVG",
            self.width,
            self.height,
            significant(self.scale),
            self.svg.markup.len()
        );
        let mut notes = approximations(&self.svg.stats);
        if self.svg.approximated > 0 {
            notes.push(format!(
                "{} drawing steps SVG cannot express",
                self.svg.approximated
            ));
        }
        if !notes.is_empty() {
            text.push_str("\napproximations: ");
            text.push_str(&notes.join(", "));
        }
        text
    }
}

/// [`render_node`] as SVG (vector paths, text as outlines, images embedded as PNG). A layer or
/// page is cropped like an export (its drawn ink, at least a frame's box), as the app and the
/// CLI export it; a node inside an instance keeps its own box. The markup is limited to `limit`
/// bytes; a larger export is an error.
pub fn svg_node(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    rendering: &Rendering,
    limit: usize,
) -> Result<SvgShot, CoreError> {
    with_target(
        doc,
        node_id,
        scale,
        |root, bounds, scale, background, max_side| {
            let (mut width, mut height, viewport) = viewport(bounds, scale, background, max_side)?;
            let scale = viewport.transform.as_coeffs()[0];
            let svg = match root {
                Root::Node(id) => {
                    let svg = rendering
                        .draw_export_svg(doc, id, scale, background, limit)
                        .ok_or_else(|| {
                            CoreError::render(format!("node {id} has nothing visible"))
                        })?;
                    let side = |v: f64| (v - 1e-6).ceil().clamp(1.0, f64::from(u16::MAX)) as u16;
                    (width, height) = (side(svg.width), side(svg.height));
                    svg
                }
                Root::Path(_) => rendering.draw_svg(doc, root, &viewport, limit),
            };
            if svg.over_limit {
                return Err(CoreError::limit(format!(
                    "the SVG is over {limit} bytes; export a smaller node"
                )));
            }
            Ok(SvgShot {
                svg,
                width,
                height,
                scale,
            })
        },
    )
}

/// Resolves `node_id` to what to draw and its crop box (page units), then calls `draw` with
/// the root, the box, the requested scale, the page background (pages only) and the side cap.
fn with_target<R>(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    draw: impl FnOnce(Root<'_>, Rect, f64, Option<Background>, f64) -> Result<R, CoreError>,
) -> Result<R, CoreError> {
    // An omitted scale gets the smaller side cap; an explicit one keeps the 4096 px cap.
    let max_side = if scale.is_some() {
        MAX_SIDE_PX
    } else {
        DEFAULT_SIDE_PX
    };
    let scale = scale.unwrap_or(1.0);
    if !SCALE_RANGE.contains(&scale) {
        return Err(CoreError::input("scale must be between 0.01 and 4"));
    }
    let id = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) => id,
        Ok(NodeId::InstancePath(path)) => {
            let bounds = instance_child_bounds(doc, &path)?;
            return draw(Root::Path(&path), bounds, scale, None, max_side);
        }
        Err(error) => return Err(error),
    };
    let Some(node) = doc.node(id) else {
        return Err(CoreError::input(format!(
            "node {id} does not exist in this file"
        )));
    };
    if node.kind == "DOCUMENT" {
        return Err(CoreError::input("pick a page or a layer, not the document"));
    }
    let Some(page) = doc.page_of(id) else {
        return Err(CoreError::input(format!("node {id} is not on a page")));
    };
    let is_page = node.kind == "CANVAS";
    let size = |axis: &str| node.data.get("size").and_then(|s| s.num(axis));
    if !is_page && size("x").unwrap_or(0.0) <= 0.0 && size("y").unwrap_or(0.0) <= 0.0 {
        return Err(CoreError::render(format!(
            "node {id} has zero size, nothing to render"
        )));
    }
    // `content_bounds` is a culling margin, not ink: it only tells hidden nodes apart and
    // gives a page (which has no size) its extent. Other nodes are cropped to their own box.
    let Some(content) = content_bounds(doc, id) else {
        return Err(CoreError::render(format!(
            "node {id} is hidden or has nothing visible"
        )));
    };
    let bounds = if is_page {
        content
    } else {
        node_bounds(doc, id).unwrap_or(content)
    };
    let background = is_page.then(|| page_background(doc, page));
    draw(Root::Node(id), bounds, scale, background, max_side)
}

type Background = figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>;

/// What a screenshot draws: a document node, or one node inside an instance on its own.
#[derive(Clone, Copy)]
pub enum Root<'p> {
    Node(Guid),
    Path(&'p InstancePath),
}

/// The box the renderer gives the instance child at `path` when drawn alone.
fn instance_child_bounds(doc: &Document, path: &InstancePath) -> Result<Rect, CoreError> {
    if let Err(error) = resolve_path(doc, path) {
        return Err(CoreError::resolve(path, error));
    }
    let Some(bounds) = Renderer::new().path_bounds(doc, path) else {
        return Err(CoreError::render(format!(
            "node {path} is hidden or has nothing visible"
        )));
    };
    if bounds.width() <= 0.0 && bounds.height() <= 0.0 {
        return Err(CoreError::render(format!(
            "node {path} has zero size, nothing to render"
        )));
    }
    Ok(bounds)
}

/// Renders a layer (not a page) cropped to its own box on a transparent background, as
/// `get_screenshot` does. Used by `get_design_context` assets.
pub fn layer_shot(
    doc: &Document,
    root: Root<'_>,
    scale: f64,
    rendering: &Rendering,
) -> Result<Shot, CoreError> {
    let bounds = match root {
        Root::Node(id) => node_bounds(doc, id),
        Root::Path(path) => Renderer::new().path_bounds(doc, path),
    }
    .ok_or_else(|| CoreError::render("hidden or nothing visible"))?;
    render(doc, root, bounds, scale, None, MAX_SIDE_PX, rendering)
}

/// Renders `root` into `bounds` (page units) at `scale`, its longest side capped at `max_side`.
fn render(
    doc: &Document,
    root: Root<'_>,
    bounds: Rect,
    scale: f64,
    background: Option<Background>,
    max_side: f64,
    rendering: &Rendering,
) -> Result<Shot, CoreError> {
    let (_, _, viewport) = viewport(bounds, scale, background, max_side)?;
    let frame = rendering.draw(doc, root, &viewport);
    Ok(Shot {
        frame,
        scale: viewport.transform.as_coeffs()[0],
    })
}

/// The output size and viewport for `bounds` (page units) at `scale`, its longest side capped
/// at `max_side`.
fn viewport(
    bounds: Rect,
    scale: f64,
    background: Option<Background>,
    max_side: f64,
) -> Result<(u16, u16, Viewport), CoreError> {
    let longest = bounds.width().max(bounds.height()) * scale;
    if !longest.is_finite() || longest <= 0.0 {
        return Err(CoreError::render("the node has no finite size to render"));
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
    Ok((width, height, viewport))
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
        (stats.emoji_approximated, "emoji approximated"),
        (stats.emoji_skipped, "emoji skipped"),
        (stats.approximated_effects, "shadow effects approximated"),
        (stats.skipped_effects, "effects skipped"),
        (stats.missing_images, "images missing"),
        (stats.images_loading, "images loading"),
        (stats.missing_symbols, "components missing"),
        (stats.unsupported_paints, "paints skipped"),
        (stats.approximated_blends, "blend modes approximated"),
        (stats.approximated_paints, "paints approximated"),
        (stats.approximated_strokes, "stroke alignments approximated"),
        (
            stats.approximated_booleans,
            "boolean operations approximated",
        ),
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
