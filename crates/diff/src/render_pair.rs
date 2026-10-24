//! Whole-page pictures stay available as folded context; screen masks are separate.
use crate::render::{ImageReader, MAX_RENDER_PIXELS, RenderImages, RenderOpts};
use crate::render_pixels::{compare, encode, encode_pixels, premul_rgba};
use crate::{RenderDiff, RenderStats};
use figstudio_fig::{Document, Guid};
use figstudio_render::{MAX_SIDE, Renderer, Viewport, content_bounds, kurbo, page_background};
pub(crate) struct Pair {
    pub(crate) diff: RenderDiff,
    pub(crate) images: RenderImages,
    pub(crate) budget: bool,
    pub(crate) max_side: bool,
}

pub(crate) fn render_pair(
    (old_renderer, new_renderer): (&mut Renderer, &mut Renderer),
    (old, new): (&Document, &Document),
    (page_old, page_new): (Option<Guid>, Option<Guid>),
    (old_read, new_read): (ImageReader<'_>, ImageReader<'_>),
    opts: &RenderOpts,
) -> Option<Pair> {
    let bounds = match (
        page_old.and_then(|id| content_bounds(old, id)),
        page_new.and_then(|id| content_bounds(new, id)),
    ) {
        (Some(a), Some(b)) => a.union(b),
        (Some(single), None) | (None, Some(single)) => single,
        (None, None) => return None,
    };
    let (effective, width, height, budget, max_side) = fit_canvas(bounds, opts.scale.max(0.01));
    let transform =
        kurbo::Affine::scale(effective) * kurbo::Affine::translate((-bounds.x0, -bounds.y0));
    let draw = |renderer: &mut Renderer, doc: &Document, page: Option<Guid>, read: ImageReader| {
        page.map(|id| {
            renderer.render_complete(
                doc,
                id,
                &Viewport {
                    width: width as u16,
                    height: height as u16,
                    transform,
                    background: Some(page_background(doc, id)),
                },
                read,
            )
        })
    };
    let before = draw(old_renderer, old, page_old, old_read);
    let after = draw(new_renderer, new, page_new, new_read);
    let bg = |doc: &Document, page: Option<Guid>| {
        page.map(|id| premul_rgba(page_background(doc, id)))
            .unwrap_or([0; 4])
    };
    let (pixels_changed, worst_channel_delta, diff_frame, _) = compare(
        &before,
        &after,
        bg(old, page_old),
        bg(new, page_new),
        width,
        height,
        opts.threshold,
    );
    Some(Pair {
        diff: RenderDiff {
            pixels_changed: Some(pixels_changed),
            worst_channel_delta: Some(worst_channel_delta),
            width: Some(width),
            height: Some(height),
            old_stats: before.as_ref().map(|frame| RenderStats::from(&frame.stats)),
            new_stats: after.as_ref().map(|frame| RenderStats::from(&frame.stats)),
            ..Default::default()
        },
        images: RenderImages {
            format: opts.format,
            before: before.as_ref().and_then(|f| encode(f, opts.format)),
            after: after.as_ref().and_then(|f| encode(f, opts.format)),
            diff: Some(encode_pixels(&diff_frame, width, height, opts.format)?),
            screens: Vec::new(),
            shared: Vec::new(),
            variants: Vec::new(),
        },
        budget,
        max_side,
    })
}

fn fit_canvas(bounds: kurbo::Rect, scale: f64) -> (f64, u32, u32, bool, bool) {
    let (bw, bh) = (bounds.width(), bounds.height());
    let side_cap = (f64::from(MAX_SIDE) - 1.0) / bw.max(bh).max(1.0);
    let max_side = scale > side_cap;
    let mut effective = scale.min(side_cap);
    let size = |e: f64| {
        (
            ((bw * e).ceil().max(1.0) as u32).min(u32::from(u16::MAX)),
            ((bh * e).ceil().max(1.0) as u32).min(u32::from(u16::MAX)),
        )
    };
    let (mut width, mut height) = size(effective);
    let mut budget = false;
    for _ in 0..8 {
        if (width as usize).saturating_mul(height as usize) <= MAX_RENDER_PIXELS {
            break;
        }
        budget = true;
        effective *= (MAX_RENDER_PIXELS as f64 / (width as f64 * height as f64)).sqrt();
        (width, height) = size(effective);
    }
    (effective, width, height, budget, max_side)
}
