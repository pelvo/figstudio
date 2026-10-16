//! Pixel comparison of changed pages on a union-bounds canvas.
use std::collections::BTreeMap;
use std::io::Cursor;

use figstudio_fig::{Document, Guid};
use figstudio_render::images::ImageRequest;
use figstudio_render::{
    Frame, MAX_SIDE, Renderer, Viewport, content_bounds, kurbo, page_background,
};
use image::{ImageBuffer, Rgba};

use crate::{DiffReport, PageStatus, RenderDiff, RenderStats};

pub const MAX_RENDER_PIXELS: usize = 16 * 1024 * 1024;
/// Pages rendered per comparison unless [`RenderOpts::max_pages`] says otherwise.
pub const MAX_RENDER_PAGES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOpts {
    pub scale: f64,
    pub threshold: u8,
    /// Lay out text with installed fonts when the file has no stored glyph outlines;
    /// matches `Renderer::system_fonts` so callers mirror the `render` command.
    pub system_fonts: bool,
    /// Render at most this many changed pages (`None`: [`MAX_RENDER_PAGES`]).
    pub max_pages: Option<usize>,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            scale: 1.0,
            threshold: 8,
            system_fonts: true,
            max_pages: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderImages {
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub diff: Option<Vec<u8>>,
}

/// Reads the bytes of an image a lazily opened document is missing (`None`: unreadable).
pub type ImageReader<'a> = &'a dyn Fn(&ImageRequest) -> Option<Vec<u8>>;

fn no_reader(_: &ImageRequest) -> Option<Vec<u8>> {
    None
}

/// Renders every changed page and keeps the PNGs of all of them in memory.
/// Lazily opened documents draw their archive images as placeholders; use
/// [`attach_render_diffs_with_sink`] to read them and to stream pages out.
#[must_use]
pub fn attach_render_diffs(
    old: &Document,
    new: &Document,
    report: &mut DiffReport,
    opts: &RenderOpts,
) -> BTreeMap<Guid, RenderImages> {
    let mut out = BTreeMap::new();
    attach_render_diffs_with_sink(
        old,
        new,
        report,
        opts,
        &no_reader,
        &no_reader,
        &mut |id, images| {
            out.insert(id, images.clone());
        },
    );
    out
}

/// Renders changed pages in report order, handing each page's PNGs to `sink` before the next
/// page renders, so only one page of pixels is alive at a time. `old_read`/`new_read` read the
/// images that `old`/`new` hold lazily. At most `opts.max_pages` pages render; the rest keep
/// `render == None` and one note in `report.truncated` counts them.
pub fn attach_render_diffs_with_sink(
    old: &Document,
    new: &Document,
    report: &mut DiffReport,
    opts: &RenderOpts,
    old_read: ImageReader<'_>,
    new_read: ImageReader<'_>,
    sink: &mut dyn FnMut(Guid, &RenderImages),
) {
    let mut old_renderer = Renderer::new();
    let mut new_renderer = Renderer::new();
    old_renderer.system_fonts = opts.system_fonts;
    new_renderer.system_fonts = opts.system_fonts;
    let cap = opts.max_pages.unwrap_or(MAX_RENDER_PAGES);
    let (mut rendered, mut skipped) = (0usize, 0usize);
    let mut notes = Vec::new();
    for page in &mut report.pages {
        if page.status == PageStatus::Unchanged {
            continue;
        }
        if rendered >= cap {
            skipped += 1;
            continue;
        }
        let (old_id, new_id) = match page.status {
            PageStatus::Added => (None, Some(page.id)),
            PageStatus::Removed => (Some(page.id), None),
            _ => (Some(page.id), Some(page.id)),
        };
        let Some(pair) = render_pair(
            (&mut old_renderer, &mut new_renderer),
            (old, new),
            (old_id, new_id),
            (old_read, new_read),
            opts,
        ) else {
            continue;
        };
        rendered += 1;
        if pair.budget {
            notes.push(format!(
                "page {} render scaled down to fit the {MAX_RENDER_PIXELS} pixel budget",
                page.id,
            ));
        }
        if pair.max_side {
            notes.push(format!(
                "page {} render scaled down to fit the max side",
                page.id
            ));
        }
        page.render = Some(pair.diff);
        sink(page.id, &pair.images);
    }
    if skipped > 0 {
        notes.push(format!(
            "render limit of {cap} pages reached; {skipped} changed pages were not rendered"
        ));
    }
    report.truncated.extend(notes);
}

#[must_use]
pub fn render_page_pair(
    old: &Document,
    new: &Document,
    page_old: Option<Guid>,
    page_new: Option<Guid>,
    opts: &RenderOpts,
) -> Option<(RenderDiff, RenderImages)> {
    render_page_pair_with_readers(old, new, page_old, page_new, opts, &no_reader, &no_reader)
}

/// [`render_page_pair`] reading the lazily held images of `old`/`new` with `old_read`/`new_read`.
#[must_use]
pub fn render_page_pair_with_readers(
    old: &Document,
    new: &Document,
    page_old: Option<Guid>,
    page_new: Option<Guid>,
    opts: &RenderOpts,
    old_read: ImageReader<'_>,
    new_read: ImageReader<'_>,
) -> Option<(RenderDiff, RenderImages)> {
    let mut old_renderer = Renderer::new();
    let mut new_renderer = Renderer::new();
    old_renderer.system_fonts = opts.system_fonts;
    new_renderer.system_fonts = opts.system_fonts;
    let pair = render_pair(
        (&mut old_renderer, &mut new_renderer),
        (old, new),
        (page_old, page_new),
        (old_read, new_read),
        opts,
    )?;
    Some((pair.diff, pair.images))
}

struct Pair {
    diff: RenderDiff,
    images: RenderImages,
    budget: bool,
    max_side: bool,
}

fn render_pair(
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
    let (pixels_changed, worst_channel_delta, diff_frame) = compare(
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
            old_png: None,
            new_png: None,
            diff_png: None,
            pixels_changed: Some(pixels_changed),
            worst_channel_delta: Some(worst_channel_delta),
            width: Some(width),
            height: Some(height),
            old_stats: before.as_ref().map(|frame| RenderStats::from(&frame.stats)),
            new_stats: after.as_ref().map(|frame| RenderStats::from(&frame.stats)),
        },
        images: RenderImages {
            before: before.as_ref().and_then(encode),
            after: after.as_ref().and_then(encode),
            diff: Some(encode_pixels(&diff_frame, width, height)?),
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

fn encode(frame: &Frame) -> Option<Vec<u8>> {
    encode_pixels(
        frame.pixmap.data_as_u8_slice(),
        u32::from(frame.pixmap.width()),
        u32::from(frame.pixmap.height()),
    )
}

fn encode_pixels(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    let mut straight = vec![0u8; pixels.len()];
    let (src, _) = pixels.as_chunks::<4>();
    let (dst, _) = straight.as_chunks_mut::<4>();
    for (src, dst) in src.iter().zip(dst) {
        let a = src[3];
        if a != 0 {
            for i in 0..3 {
                dst[i] = u8::try_from((u32::from(src[i]) * 255 + u32::from(a) / 2) / u32::from(a))
                    .unwrap_or(u8::MAX);
            }
            dst[3] = a;
        }
    }
    let buffer = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, straight)?;
    let mut out = Vec::new();
    buffer
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

fn pixel4(px: &[u8], at: usize) -> [u8; 4] {
    px.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .unwrap_or([0; 4])
}

fn channel_delta(a: [u8; 4], b: [u8; 4]) -> u8 {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(y))
        .max()
        .unwrap_or(0)
}

fn premul_rgba(
    color: figstudio_render::color::AlphaColor<figstudio_render::color::Srgb>,
) -> [u8; 4] {
    let p = color.premultiply().to_rgba8();
    [p.r, p.g, p.b, p.a]
}

fn compare(
    before: &Option<Frame>,
    after: &Option<Frame>,
    old_bg: [u8; 4],
    new_bg: [u8; 4],
    width: u32,
    height: u32,
    threshold: u8,
) -> (u64, u8, Vec<u8>) {
    let count = (width as usize) * (height as usize);
    let mut diff = vec![0u8; count * 4];
    let mut changed = 0u64;
    let mut worst = 0u8;
    for i in 0..count {
        let at = i * 4;
        let px = |frame: &Frame| pixel4(frame.pixmap.data_as_u8_slice(), at);
        let base = match (after, before) {
            (Some(frame), _) | (None, Some(frame)) => px(frame),
            (None, None) => [0; 4],
        };
        let delta = match (before, after) {
            (Some(old), Some(new)) => channel_delta(px(old), px(new)),
            (Some(frame), None) => channel_delta(px(frame), old_bg),
            (None, Some(frame)) => channel_delta(px(frame), new_bg),
            (None, None) => 0,
        };
        worst = worst.max(delta);
        if delta > threshold {
            changed += 1;
            diff[at..at + 4].copy_from_slice(&[255, 0, 0, 255]);
        } else {
            let gray =
                u8::try_from((u16::from(base[0]) + u16::from(base[1]) + u16::from(base[2])) / 3)
                    .unwrap_or(0);
            diff[at..at + 4].copy_from_slice(&[gray, gray, gray, base[3]]);
        }
    }
    (changed, worst, diff)
}
