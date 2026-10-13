//! A PNG render of a page rectangle: the crop behind the app's Copy region as PNG and the
//! region links of `get_screenshot`. Both call [`viewport`], so a link renders the area the app
//! showed.
use crate::{
    rendering::Rendering,
    screenshot::{DEFAULT_SIDE_PX, Root, SCALE_RANGE, Shot},
};
use figstudio_fig::{Document, Guid};
use figstudio_render::{Viewport, kurbo::Affine, page_background};

/// Longest side of a region image, in pixels.
pub const MAX_SIDE: f64 = 4096.0;
/// Most pixels in a region image.
pub const MAX_PIXELS: f64 = 16_000_000.0;

/// The output scale and pixel size of a `w` × `h` area drawn at `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plan {
    pub scale: f64,
    pub width: u16,
    pub height: u16,
}

/// `scale` stepped down until the image fits [`MAX_SIDE`] and [`MAX_PIXELS`]. `None` for an area
/// or scale that is not a positive finite number.
pub fn plan(w: f64, h: f64, scale: f64) -> Option<Plan> {
    plan_within(w, h, scale, MAX_SIDE)
}

/// [`plan`] with the longest side limited to `max_side` (at most [`MAX_SIDE`]) instead.
pub fn plan_within(w: f64, h: f64, scale: f64, max_side: f64) -> Option<Plan> {
    let max_side = max_side.min(MAX_SIDE);
    let ok = |v: f64| v.is_finite() && v > 0.0;
    if !(ok(w) && ok(h) && ok(scale)) {
        return None;
    }
    let mut s = scale
        .max(0.01)
        .min(max_side / w)
        .min(max_side / h)
        .min((MAX_PIXELS / (w * h)).sqrt());
    let px = |v: f64, s: f64| (v * s - 1e-6).ceil().max(1.0);
    // Rounding up a side can tip the total over the cap by a sliver: shrink until it fits.
    for _ in 0..200 {
        let (pw, ph) = (px(w, s), px(h, s));
        if pw <= max_side && ph <= max_side && pw * ph <= MAX_PIXELS {
            return Some(Plan {
                scale: s,
                width: pw as u16,
                height: ph as u16,
            });
        }
        s *= 0.999;
    }
    None
}

/// The viewport that shows `rect` (`[x, y, w, h]`, page units) of `page` at `scale`, on the
/// page background, within the size caps. `None` for an empty or non-finite area or scale.
pub fn viewport(
    doc: &Document,
    page: Guid,
    rect: [f64; 4],
    scale: f64,
) -> Option<(Viewport, Plan)> {
    viewport_within(doc, page, rect, scale, MAX_SIDE)
}

/// [`viewport`] with the longest side limited to `max_side`.
pub fn viewport_within(
    doc: &Document,
    page: Guid,
    rect: [f64; 4],
    scale: f64,
    max_side: f64,
) -> Option<(Viewport, Plan)> {
    let p = plan_within(rect[2], rect[3], scale, max_side)?;
    let viewport = Viewport {
        width: p.width,
        height: p.height,
        transform: Affine::scale(p.scale) * Affine::translate((-rect[0], -rect[1])),
        background: Some(page_background(doc, page)),
    };
    Some((viewport, p))
}

/// The message for a rect that is not a region.
pub const BAD_RECT: &str =
    "rect must be [x, y, width, height] in page units: finite numbers, width and height above 0";

/// Renders `rect` of `page`. Without `scale` it is 1, and the longest side stays within
/// [`DEFAULT_SIDE_PX`]; an explicit one is capped by [`plan`] alone.
///
/// # Errors
/// A message when `page` is not a page of the file, `scale` is outside 0.01 to 4, or the rect
/// is empty or not finite.
pub fn region_shot(
    doc: &Document,
    page: Guid,
    rect: [f64; 4],
    scale: Option<f64>,
    rendering: &Rendering,
) -> Result<Shot, String> {
    if doc.node(page).is_none_or(|n| n.kind != "CANVAS") {
        return Err(format!("page {page} does not exist in this file"));
    }
    let asked = scale.unwrap_or(1.0);
    if !SCALE_RANGE.contains(&asked) {
        return Err("scale must be between 0.01 and 4".into());
    }
    let rect = crate::link::validate_rect(rect).map_err(|_| BAD_RECT.to_string())?;
    // Like a node: without `scale` the smaller side cap applies; an explicit one gets 4096 px.
    let max_side = if scale.is_some() {
        MAX_SIDE
    } else {
        DEFAULT_SIDE_PX
    };
    let (viewport, plan) =
        viewport_within(doc, page, rect, asked, max_side).ok_or("the region has no size")?;
    let frame = rendering.draw(doc, Root::Node(page), &viewport);
    Ok(Shot {
        frame,
        scale: plan.scale,
    })
}
