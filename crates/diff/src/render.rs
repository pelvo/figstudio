//! Pixel comparison of changed pages on a union-bounds canvas.
use std::collections::BTreeMap;

use crate::render_pair::render_pair;
use crate::{DiffReport, PageStatus, RenderDiff};
use figstudio_fig::{Document, Guid};
use figstudio_render::Renderer;
use figstudio_render::images::{ImageRequest, Images};

pub const MAX_RENDER_PIXELS: usize = 16 * 1024 * 1024;
/// Pages rendered per comparison unless [`RenderOpts::max_pages`] says otherwise.
pub const MAX_RENDER_PAGES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderOpts {
    pub scale: f64,
    /// Lossless format for all report images.
    pub format: figstudio_render::encoding::ImageFormat,
    pub threshold: u8,
    /// Lay out text with installed fonts when the file has no stored glyph outlines;
    /// matches `Renderer::system_fonts` so callers mirror the `render` command.
    pub system_fonts: bool,
    /// Render at most this many changed pages (`None`: [`MAX_RENDER_PAGES`]).
    pub max_pages: Option<usize>,
    /// Work caps for standalone screens.
    pub screen_limits: crate::ScreenLimits,
    /// Decoded-image cache bytes of each of the two renderers (default
    /// [`DEFAULT_IMAGE_CACHE_BYTES`]; the renderer's own default is 1 GiB per side).
    pub image_cache_bytes: usize,
}

/// Image cache per renderer during a comparison: it draws one page at a time and the
/// cache only saves repeat decodes, so it stays far below the viewer's 1 GiB.
pub const DEFAULT_IMAGE_CACHE_BYTES: usize = 128 * 1024 * 1024;

/// A renderer for one side of a comparison, honouring the font and cache options.
pub(crate) fn side_renderer(opts: &RenderOpts) -> Renderer {
    let mut renderer = Renderer::with_images(Images::new(opts.image_cache_bytes));
    renderer.system_fonts = opts.system_fonts;
    renderer
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            scale: 1.0,
            format: figstudio_render::encoding::ImageFormat::Png,
            threshold: 8,
            system_fonts: true,
            max_pages: None,
            screen_limits: crate::ScreenLimits::default(),
            image_cache_bytes: DEFAULT_IMAGE_CACHE_BYTES,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderImages {
    /// Encoding used by every image in this page batch.
    pub format: figstudio_render::encoding::ImageFormat,
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub diff: Option<Vec<u8>>,
    /// Whole-screen pictures and their region crops, in PageDiff::screens order.
    pub screens: Vec<ScreenImages>,
    /// Component pictures of shared-change cards `(group id, images)`; only in the batch
    /// handed to the sink under [`SHARED_IMAGES_ID`], after every page.
    pub shared: Vec<(usize, RegionImages)>,
    /// Pictures of other variants of a shared component set `(group id, variant index,
    /// images)`, only the ones that look different from the shown one; same batch as `shared`.
    pub variants: Vec<(usize, usize, RegionImages)>,
}

/// Sink id of the final batch holding the shared-change component pictures; no page has it.
pub const SHARED_IMAGES_ID: Guid = Guid(u32::MAX, u32::MAX);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenImages {
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
    pub regions: Vec<RegionImages>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionImages {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

/// Reads the bytes of an image a lazily opened document is missing (`None`: unreadable).
pub type ImageReader<'a> = &'a dyn Fn(&ImageRequest) -> Option<Vec<u8>>;

fn no_reader(_: &ImageRequest) -> Option<Vec<u8>> {
    None
}

/// Renders every changed page and keeps the images of all of them in memory.
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

/// Renders changed pages in report order, handing each page's images to `sink` before the next
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
    let mut old_renderer = side_renderer(opts);
    let mut new_renderer = side_renderer(opts);
    let cap = opts.max_pages.unwrap_or(MAX_RENDER_PAGES);
    let (mut rendered, mut skipped) = (0usize, 0usize);
    let mut notes = Vec::new();
    let mut doc_pixels = 0u64;
    let mut groups = crate::shared::SharedGroups::default();
    report.shared_changes.clear();
    let mut masters = crate::shared_component::Masters::new(
        (old, new),
        report
            .pages
            .iter()
            .flat_map(|p| {
                let added = p.added.iter().map(|n| n.id);
                added
                    .chain(p.removed.iter().map(|n| n.id))
                    .chain(p.changed.iter().map(|n| n.id))
            })
            .collect(),
    );
    for (page_index, page) in report.pages.iter_mut().enumerate() {
        for region in page.screens.iter_mut().flat_map(|s| &mut s.regions) {
            region.shared = None;
        }
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
        let mut images = pair.images;
        let signatures = crate::render_screens::attach_screens(
            (&mut old_renderer, &mut new_renderer),
            &mut masters,
            page,
            (old_read, new_read),
            (opts, &mut doc_pixels),
            &mut images,
            &mut notes,
        );
        groups.record(page_index, signatures);
        page.render = Some(pair.diff);
        sink(page.id, &images);
    }
    if skipped > 0 {
        notes.push(format!(
            "render limit of {cap} pages reached; {skipped} changed pages were not rendered"
        ));
    }
    report.truncated.extend(notes);
    groups.finish(report, (old, new));
    if let Some(images) = crate::render_shared::component_views(
        (&mut old_renderer, &mut new_renderer),
        (old, new),
        (old_read, new_read),
        opts,
        report,
    ) {
        sink(SHARED_IMAGES_ID, &images);
    }
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
    let mut old_renderer = side_renderer(opts);
    let mut new_renderer = side_renderer(opts);
    let pair = render_pair(
        (&mut old_renderer, &mut new_renderer),
        (old, new),
        (page_old, page_new),
        (old_read, new_read),
        opts,
    )?;
    Some((pair.diff, pair.images))
}
