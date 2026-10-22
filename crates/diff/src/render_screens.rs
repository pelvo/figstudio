//! Standalone screen comparison; canvas placement never enters the pixel mask.
use crate::screen_limits::PageTally;
use crate::shared::RegionKey;
use crate::shared_component::Masters;
use crate::{PageDiff, RenderImages, RenderOpts, ScreenDiff, ScreenImages, ScreenStatus};
use crate::{
    render::{ImageReader, MAX_RENDER_PIXELS},
    render_pixels::{compare_mask, encode},
    screen_bounds,
    screens::{ScreenPair, match_screens},
};
use figstudio_fig::{Document, Guid};
use figstudio_render::{Renderer, Viewport, absolute_transform, geom::size, kurbo::Affine};

const SCREEN_PIXELS: usize = 4_194_304;

pub(crate) struct ScreenRender {
    pub(crate) screen: ScreenDiff,
    pub(crate) pictures: ScreenImages,
    signatures: Vec<crate::shared::Signature>,
    transforms: (Affine, Affine),
    mask: Vec<bool>,
    capped: bool,
}

pub(crate) fn attach_screens(
    renderers: (&mut Renderer, &mut Renderer),
    masters: &mut Masters,
    page: &mut PageDiff,
    readers: (ImageReader<'_>, ImageReader<'_>),
    (opts, doc_pixels): (&RenderOpts, &mut u64),
    images: &mut RenderImages,
    notes: &mut Vec<String>,
) -> Vec<Vec<RegionKey>> {
    let docs = masters.docs();
    let mut signatures = Vec::new();
    let (old_renderer, new_renderer) = renderers;
    let lines = crate::html_changes::page_lines(page);
    let mut crop_budget = MAX_RENDER_PIXELS;
    let pending = std::mem::take(&mut page.screens);
    page.screens_unchanged = 0;
    page.screens_moved = 0;
    let pairs = match_screens(docs.0, docs.1, page.id);
    let mut capped_any = false;
    let mut tally = PageTally::new(opts.screen_limits, doc_pixels);
    tally.keep_page(images);
    let members: Vec<_> = pairs
        .iter()
        .map(|&pair| screen_bounds::members(page, docs, pair))
        .collect();
    let has_lines: Vec<bool> = pairs
        .iter()
        .zip(&members)
        .map(|(pair, nodes)| {
            lines.items.iter().any(|l| {
                nodes.contains(&l.id)
                    && crate::screen_model::counts(l, pair.matched_by, (pair.old, pair.new))
            })
        })
        .collect();
    // Draw order: screens with change lines, then screens with other edits inside them (a
    // moved layer has no line), then the rest, which are usually unchanged: edits to the
    // screen root alone (a canvas move, a recreated screen's new ids) do not change its own
    // picture. Screens that turn out unchanged cannot use up the screen caps first, and the
    // report keeps document order.
    let inner = |i: usize| {
        members[i]
            .iter()
            .any(|id| Some(*id) != pairs[i].old && Some(*id) != pairs[i].new)
    };
    let tier = |i: usize| match (has_lines[i], inner(i)) {
        (true, _) => 0,
        (false, true) => 1,
        (false, false) => 2,
    };
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    order.sort_by_key(|&i| (tier(i), i));
    // Only screens present on both sides crop. Each tier splits what is left of the budget
    // among its own screens, so likely-unchanged screens drawn later cannot dilute the share
    // of a known change; their unused shares carry forward.
    let crops = |pair: &ScreenPair| pair.old.is_some() && pair.new.is_some();
    let mut croppers_left = [0usize; 3];
    for (i, pair) in pairs.iter().enumerate() {
        croppers_left[tier(i)] += usize::from(crops(pair));
    }
    let mut kept: Vec<(usize, ScreenDiff, ScreenImages, Vec<RegionKey>)> = Vec::new();
    for index in order {
        let pair = pairs[index];
        let Some(node) = pair
            .new
            .and_then(|id| docs.1.node(id))
            .or_else(|| pair.old.and_then(|id| docs.0.node(id)))
        else {
            continue;
        };
        let moved = crate::screen_model::moved(docs, pair);
        let nodes = members[index].clone();
        let (_, w, h, _) = dimensions(docs, pair);
        let over = !tally.admit(u64::from(w) * u64::from(h) * 2);
        let share = if crops(&pair) {
            let left = &mut croppers_left[tier(index)];
            let share = crop_budget / (*left).max(1);
            *left = left.saturating_sub(1);
            share
        } else {
            0
        };
        let mut available = share;
        let rendered = if over {
            None
        } else {
            render_screen(
                (&mut *old_renderer, &mut *new_renderer),
                docs,
                pair,
                readers,
                opts,
                &mut available,
                false,
            )
        };
        let Some(result) = rendered else {
            if !over {
                notes.push(format!("page {} screen rendering unavailable", page.id));
            }
            if let Some(screen) = pending
                .iter()
                .find(|s| s.old_id == pair.old && s.new_id == pair.new)
            {
                kept.push((index, screen.clone(), ScreenImages::default(), Vec::new()));
            }
            continue;
        };
        let ScreenRender {
            mut screen,
            pictures,
            signatures: region_signatures,
            transforms,
            mask,
            capped,
        } = result;
        crop_budget -= share - available;
        capped_any |= capped;
        screen.moved = moved;
        for region in &mut screen.regions {
            region.nodes = nodes
                .iter()
                .copied()
                .filter(|id| region.nodes.contains(id))
                .collect();
            if region.nodes.is_empty() {
                region.nodes = screen_bounds::containing_leaves(
                    region,
                    &nodes,
                    docs,
                    pair,
                    (transforms, (&mask, screen.width as usize)),
                );
            }
        }
        screen.nodes = nodes;
        let components = masters.keys(
            (pair.old, pair.new),
            transforms,
            &screen.regions,
            (&mask, screen.width as usize),
        );
        let region_keys: Vec<_> = region_signatures
            .into_iter()
            .zip(components)
            .map(|(signature, component)| RegionKey {
                signature,
                component,
            })
            .collect();
        if pair.old.is_some()
            && pair.new.is_some()
            && screen.pixels_changed == Some(0)
            && !has_lines[index]
        {
            page.screens_unchanged += 1;
            page.screens_moved += usize::from(moved);
            continue;
        }
        // Only pictures that stay in `images` count toward the page byte cap.
        tally.keep(&pictures);
        screen.name.clone_from(&node.name);
        screen.kind.clone_from(&node.kind);
        kept.push((index, screen, pictures, region_keys));
    }
    kept.sort_by_key(|(index, ..)| *index);
    for (_, screen, pictures, region_keys) in kept {
        page.screens.push(screen);
        images.screens.push(pictures);
        signatures.push(region_keys);
    }
    notes.extend(tally.note(page.id));
    if capped_any {
        notes.push(format!(
            "page {} screen/crops capped by side, pixel budget or region limit",
            page.id
        ));
    }
    signatures
}

pub(crate) fn dimensions(docs: (&Document, &Document), pair: ScreenPair) -> (f64, u32, u32, bool) {
    let sizes = [(docs.0, pair.old), (docs.1, pair.new)].map(|(doc, id)| {
        id.and_then(|i| doc.node(i))
            .map_or((0.0, 0.0), |n| size(&n.data))
    });
    let (w, h) = (
        sizes[0].0.max(sizes[1].0).max(1.0),
        sizes[0].1.max(sizes[1].1).max(1.0),
    );
    let mut scale = 1.0_f64.min((f64::from(figstudio_render::MAX_SIDE) - 1.0) / w.max(h));
    let dims = |s: f64| {
        (
            (w * s).ceil().max(1.0) as u32,
            (h * s).ceil().max(1.0) as u32,
        )
    };
    let (mut width, mut height) = dims(scale);
    while width as usize * height as usize > SCREEN_PIXELS {
        scale *= (SCREEN_PIXELS as f64 / (width as f64 * height as f64)).sqrt() * 0.999999;
        (width, height) = dims(scale);
    }
    (scale, width, height, scale < 1.0)
}

/// `merge` folds all changed areas into one box (a component's single Before | After).
pub(crate) fn render_screen(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    pair: ScreenPair,
    readers: (ImageReader<'_>, ImageReader<'_>),
    opts: &RenderOpts,
    remaining: &mut usize,
    merge: bool,
) -> Option<ScreenRender> {
    let (scale, width, height, mut capped) = dimensions(docs, pair);
    let local = |doc: &Document, id: Option<Guid>| {
        id.map_or(Affine::IDENTITY, |i| absolute_transform(doc, i).inverse())
    };
    let transforms = (
        Affine::scale(scale) * local(docs.0, pair.old),
        Affine::scale(scale) * local(docs.1, pair.new),
    );
    if transforms
        .0
        .as_coeffs()
        .iter()
        .chain(transforms.1.as_coeffs().iter())
        .any(|v| !v.is_finite())
    {
        return None;
    }
    let draw = |renderer: &mut Renderer,
                doc: &Document,
                id: Option<Guid>,
                read: ImageReader,
                transform| {
        id.map(|id| {
            renderer.render_complete(
                doc,
                id,
                &Viewport {
                    width: width as u16,
                    height: height as u16,
                    transform,
                    background: None,
                },
                read,
            )
        })
    };
    let before = draw(renderers.0, docs.0, pair.old, readers.0, transforms.0);
    let after = draw(renderers.1, docs.1, pair.new, readers.1, transforms.1);
    let mask = compare_mask(
        &before,
        &after,
        ([0; 4], [0; 4]),
        (width, height),
        opts.threshold,
    );
    let mut regions = crate::regions::all_regions(&mask, width as usize, height as usize);
    // Every changed pixel, including specks left out of the areas: a screen whose only
    // change is small must not be counted as unchanged.
    let pixels = mask.iter().filter(|&&changed| changed).count() as u64;
    if merge {
        regions = crate::regions::union(regions);
    }
    capped |= regions.len() > crate::MAX_REGIONS;
    regions.truncate(crate::MAX_REGIONS);
    let mut crops = Vec::new();
    if let (Some(a), Some(b)) = (pair.old, pair.new) {
        let result = crate::render_crops::render_crops(
            renderers,
            docs,
            (a, b),
            readers,
            regions,
            (transforms, scale, opts.format, (width, height)),
            remaining,
        );
        regions = result.0;
        crops = result.1;
        capped |= result.2;
    } else {
        regions.clear();
    }
    let signatures = crate::shared::signatures(&regions, &mask, width, &before, &after);
    // Membership is restricted to each side's subtree, never to overlapping screens.
    let ids = crate::screen_bounds::subtree(docs.0, pair.old)
        .union(&crate::screen_bounds::subtree(docs.1, pair.new))
        .copied()
        .collect::<Vec<_>>();
    screen_bounds::region_nodes(&mut regions, &ids, docs, pair, transforms, (width, height));
    let status = match (pair.old, pair.new) {
        (None, _) => ScreenStatus::Added,
        (_, None) => ScreenStatus::Removed,
        _ => ScreenStatus::Changed,
    };
    Some(ScreenRender {
        screen: ScreenDiff {
            old_id: pair.old,
            new_id: pair.new,
            name: String::new(),
            kind: String::new(),
            status,
            matched_by: pair.matched_by,
            moved: false,
            width,
            height,
            pixels_changed: Some(pixels),
            thumb_old: None,
            thumb_new: None,
            regions,
            nodes: Vec::new(),
        },
        pictures: ScreenImages {
            before: before.as_ref().and_then(|f| encode(f, opts.format)),
            after: after.as_ref().and_then(|f| encode(f, opts.format)),
            regions: crops,
        },
        signatures,
        transforms,
        mask,
        capped,
    })
}
