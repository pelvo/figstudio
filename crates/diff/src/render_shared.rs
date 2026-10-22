//! Shared-change cards of a changed component show the component itself, drawn standalone.
use crate::render::{ImageReader, MAX_RENDER_PIXELS};
use crate::render_screens::{dimensions, render_screen};
use crate::screens::{MatchBy, ScreenPair};
use crate::{DiffReport, Region, RegionImages, RenderImages, RenderOpts};
use figstudio_fig::{Document, Guid};
use figstudio_render::Renderer;

type Readers<'a> = (ImageReader<'a>, ImageReader<'a>);

/// Component groups drawn standalone per comparison; the rest keep their region crop.
pub const MAX_COMPONENT_VIEWS: usize = 64;
/// Pixels (both sides) of every standalone component and variant picture together.
pub const MAX_COMPONENT_VIEW_PIXELS: usize = 4 * MAX_RENDER_PIXELS;

/// What the standalone views have used so far; the budget is global, not per card.
#[derive(Default)]
struct Spent {
    pixels: usize,
    views: usize,
    skipped: usize,
}

/// One component drawn standalone in both files; `None` unless it is a SYMBOL in both.
/// Every view (a main component or a variant) counts toward both limits.
fn draw(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    readers: Readers<'_>,
    opts: &RenderOpts,
    spent: &mut Spent,
    id: Guid,
) -> Option<(Region, RegionImages)> {
    let symbol = |doc: &Document| doc.node(id).is_some_and(|n| n.kind == "SYMBOL");
    if !(symbol(docs.0) && symbol(docs.1)) {
        return None;
    }
    let pair = ScreenPair {
        old: Some(id),
        new: Some(id),
        matched_by: Some(MatchBy::Id),
    };
    let (_, w, h, _) = dimensions(docs, pair);
    let frame = w as usize * h as usize * 2;
    if spent.views >= MAX_COMPONENT_VIEWS || spent.pixels + frame > MAX_COMPONENT_VIEW_PIXELS {
        spent.skipped += 1;
        return None;
    }
    spent.views += 1;
    let mut budget = MAX_RENDER_PIXELS;
    let done = render_screen(renderers, docs, pair, readers, opts, &mut budget, true);
    // The frame pictures plus the crops actually cut from them.
    spent.pixels += frame + (MAX_RENDER_PIXELS - budget);
    let mut done = done?;
    let region = done
        .screen
        .regions
        .pop()
        .filter(|_| done.pictures.regions.len() == 1)?;
    Some((region, done.pictures.regions.pop()?))
}

/// Renders each shared change's component once when it exists in both files, sets the card's
/// `component_view` and picture size, and returns the pictures (`None`: no card has one). Other
/// variants of a component set are drawn too, but kept only when they look different.
pub(crate) fn component_views(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    readers: Readers<'_>,
    opts: &RenderOpts,
    report: &mut DiffReport,
) -> Option<RenderImages> {
    let (old_renderer, new_renderer) = renderers;
    let mut images = RenderImages {
        format: opts.format,
        ..RenderImages::default()
    };
    let mut spent = Spent::default();
    for group in &mut report.shared_changes {
        let Some(id) = group.component.as_ref().map(|c| c.id) else {
            continue;
        };
        let Some((region, pictures)) = draw(
            (&mut *old_renderer, &mut *new_renderer),
            docs,
            readers,
            opts,
            &mut spent,
            id,
        ) else {
            continue;
        };
        group.component_view = true;
        group.image_width = region.image_width;
        group.image_height = region.image_height;
        let mut kept: Vec<RegionImages> = vec![pictures.clone()];
        let variants = group.component.as_ref().map_or(0, |c| c.variants.len());
        for j in 1..variants {
            let variant = group.component.as_ref().map(|c| c.variants[j].id);
            let Some((region, pictures)) = variant.and_then(|v| {
                draw(
                    (&mut *old_renderer, &mut *new_renderer),
                    docs,
                    readers,
                    opts,
                    &mut spent,
                    v,
                )
            }) else {
                continue;
            };
            if kept.contains(&pictures) {
                continue; // looks like a picture already shown
            }
            if let Some(v) = group.component.as_mut().map(|c| &mut c.variants[j]) {
                v.image_width = region.image_width;
                v.image_height = region.image_height;
            }
            images.variants.push((group.id, j, pictures.clone()));
            kept.push(pictures);
        }
        images.shared.push((group.id, kept.swap_remove(0)));
    }
    if spent.skipped > 0 {
        report.truncated.push(format!(
            "component view limit reached ({MAX_COMPONENT_VIEWS} components or \
             {MAX_COMPONENT_VIEW_PIXELS} pixels); {} component views were not drawn: \
             a card without its view shows its changed-area crop, or is left out when no \
             crop was kept, and a variant without one is left out",
            spent.skipped
        ));
    }
    (!images.shared.is_empty()).then_some(images)
}
