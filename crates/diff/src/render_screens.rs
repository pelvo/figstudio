//! Standalone screen comparison; canvas placement never enters the pixel mask.
use crate::screen_limits::PageTally;
use crate::shared::RegionKey;
use crate::shared_component::Masters;
use crate::{PageDiff, RenderImages, RenderOpts, ScreenDiff, ScreenImages, ScreenStatus};
use crate::{
    render::{ImageReader, MAX_RENDER_PIXELS},
    render_pixels::{compare_mask, encode},
    screen_bounds,
    screens::{MatchBy, ScreenPair, match_screens},
};
use figstudio_fig::{Document, Guid};
use figstudio_render::{Renderer, Viewport, absolute_transform, geom::size, kurbo::Affine};
use fixedbitset::FixedBitSet;

const SCREEN_PIXELS: usize = 4_194_304;

pub(crate) struct ScreenRender {
    pub(crate) screen: ScreenDiff,
    pub(crate) pictures: ScreenImages,
    signatures: Vec<crate::shared::Signature>,
    transforms: (Affine, Affine),
    mask: Vec<bool>,
    capped: bool,
}

/// Whether a root edit can change the screen's own picture. The screen is drawn in its own
/// frame, so the root's whole `transform` (move, rotation, flip, scale) cancels out, and so
/// does `parentIndex`. A rename, description, export settings or constraints cannot draw
/// either; a paint transform, size, opacity or effect can.
fn can_redraw_root(field: &str) -> bool {
    let head = field.split('.').next().unwrap_or(field);
    !matches!(
        head,
        "transform"
            | "parentIndex"
            | "name"
            | "description"
            | "exportSettings"
            | "constraints"
            | "horizontalConstraint"
            | "verticalConstraint"
    )
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
    let mut crops_left_out = false;
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
    // First pass: draw each screen and find what changed. The screen caps are spent in draw
    // order, so screens with change lines go first, then screens with an edit that can change
    // their own picture (a layer inside it, a resized root, a recreated screen), then the
    // rest. Only the caps depend on this guess; crops are shared by what was drawn.
    let root = |i: usize, id: &Guid| Some(*id) == pairs[i].old || Some(*id) == pairs[i].new;
    let root_redrawn = |id: &Guid| {
        page.changed
            .iter()
            .find(|n| n.id == *id)
            .is_some_and(|n| n.changes.iter().any(|c| can_redraw_root(&c.field)))
    };
    let drawing = |i: usize| {
        pairs[i].matched_by == Some(MatchBy::NameSize)
            || members[i].iter().any(|id| !root(i, id) || root_redrawn(id))
    };
    let mut order: Vec<usize> = (0..pairs.len()).collect();
    order.sort_by_key(|&i| (!has_lines[i], !drawing(i), i));
    let mut drawn: Vec<(usize, DrawnScreen)> = Vec::new();
    let mut kept: Vec<(usize, ScreenDiff, ScreenImages, Vec<RegionKey>)> = Vec::new();
    for index in order {
        let pair = pairs[index];
        let exists = |doc: &Document, id: Option<Guid>| id.and_then(|i| doc.node(i)).is_some();
        if !exists(docs.1, pair.new) && !exists(docs.0, pair.old) {
            continue;
        }
        let (_, w, h, _) = dimensions(docs, pair);
        let over = !tally.admit(u64::from(w) * u64::from(h) * 2);
        let screen = if over {
            None
        } else {
            draw_screen(
                (&mut *old_renderer, &mut *new_renderer),
                docs,
                pair,
                readers,
                opts,
                false,
            )
        };
        let Some(screen) = screen else {
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
        if pair.old.is_some() && pair.new.is_some() && screen.pixels == 0 && !has_lines[index] {
            page.screens_unchanged += 1;
            page.screens_moved += usize::from(crate::screen_model::moved(docs, pair));
            continue;
        }
        // Only pictures that stay in `images` count toward the page byte cap.
        tally.keep(&screen.pictures);
        drawn.push((index, screen));
    }
    // Second pass, in document order: only screens with changed areas on both sides cut
    // crops, so they split the crop budget among themselves; unused shares carry forward.
    drawn.sort_by_key(|(index, _)| *index);
    let mut croppers_left = drawn
        .iter()
        .filter(|(index, screen)| crops(pairs[*index], screen))
        .count();
    for (index, screen) in drawn {
        let pair = pairs[index];
        let cuts = crops(pair, &screen);
        croppers_left -= usize::from(cuts);
        // Crop pictures count toward the page byte cap too: once it is reached, later
        // screens keep their whole-screen pictures and their areas but cut no crops.
        let full = cuts && tally.bytes_full();
        crops_left_out |= full;
        let share = if cuts && !full {
            crop_budget / (croppers_left + 1)
        } else {
            0
        };
        let mut available = share;
        let ScreenRender {
            mut screen,
            mut pictures,
            signatures: region_signatures,
            transforms,
            mask,
            capped,
        } = finish_screen(
            (&mut *old_renderer, &mut *new_renderer),
            docs,
            pair,
            readers,
            opts,
            (!full).then_some(&mut available),
            screen,
        );
        crop_budget -= share - available;
        capped_any |= capped;
        let regions = std::mem::take(&mut pictures.regions);
        let crops_only = ScreenImages {
            regions,
            ..ScreenImages::default()
        };
        tally.keep(&crops_only);
        pictures.regions = crops_only.regions;
        screen.moved = crate::screen_model::moved(docs, pair);
        let nodes = members[index].clone();
        for region in &mut screen.regions {
            // Layers mostly inside the area, plus the leaf layers that own its changed pixels
            // (a small change inside a wider layer), in member order.
            let owners = screen_bounds::containing_leaves(
                region,
                &nodes,
                docs,
                pair,
                (transforms, (&mask, screen.width as usize)),
            );
            region.nodes = nodes
                .iter()
                .copied()
                .filter(|id| region.nodes.contains(id) || owners.contains(id))
                .collect();
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
        if let Some(node) = pair
            .new
            .and_then(|id| docs.1.node(id))
            .or_else(|| pair.old.and_then(|id| docs.0.node(id)))
        {
            screen.name.clone_from(&node.name);
            screen.kind.clone_from(&node.kind);
        }
        kept.push((index, screen, pictures, region_keys));
    }
    kept.sort_by_key(|(index, ..)| *index);
    for (_, screen, pictures, region_keys) in kept {
        page.screens.push(screen);
        images.screens.push(pictures);
        signatures.push(region_keys);
    }
    notes.extend(tally.note(page.id));
    if crops_left_out {
        notes.push(format!(
            "page {} picture byte limit reached; later screens show their areas without crops",
            page.id
        ));
    }
    if capped_any {
        notes.push(format!(
            "page {} screen/crops capped by side, pixel budget or region limit",
            page.id
        ));
    }
    signatures
}

/// Whether a drawn screen cuts crops: both sides exist and something changed.
fn crops(pair: ScreenPair, screen: &DrawnScreen) -> bool {
    pair.old.is_some() && pair.new.is_some() && !screen.regions.is_empty()
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

/// A screen drawn on both sides and compared, before any crop is cut.
pub(crate) struct DrawnScreen {
    scale: f64,
    width: u32,
    height: u32,
    capped: bool,
    transforms: (Affine, Affine),
    /// The changed-pixel mask, one bit per pixel: every changed screen is held between the
    /// two passes, so it is packed (a bool per pixel would be eight times larger).
    mask: FixedBitSet,
    /// Changed areas, before crops are cut; `signatures` has one per area.
    regions: Vec<crate::Region>,
    signatures: Vec<crate::shared::Signature>,
    /// Every changed pixel, including specks left out of the areas.
    pixels: u64,
    /// The whole-screen pictures; crops are added by [`finish_screen`].
    pictures: ScreenImages,
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
    let (old, new) = renderers;
    let drawn = draw_screen((&mut *old, &mut *new), docs, pair, readers, opts, merge)?;
    Some(finish_screen(
        (old, new),
        docs,
        pair,
        readers,
        opts,
        Some(remaining),
        drawn,
    ))
}

/// Draws both sides of a screen in its own frame and finds the changed areas. The frames
/// are dropped once the pictures are encoded and the areas' signatures taken.
fn draw_screen(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    pair: ScreenPair,
    readers: (ImageReader<'_>, ImageReader<'_>),
    opts: &RenderOpts,
    merge: bool,
) -> Option<DrawnScreen> {
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
    if pair.old.is_none() || pair.new.is_none() {
        regions.clear();
    }
    let signatures = crate::shared::signatures(&regions, &mask, width, &before, &after);
    let mut bits = FixedBitSet::with_capacity(mask.len());
    for (i, _) in mask.iter().enumerate().filter(|(_, changed)| **changed) {
        bits.insert(i);
    }
    Some(DrawnScreen {
        scale,
        width,
        height,
        capped,
        transforms,
        regions,
        signatures,
        pixels,
        mask: bits,
        pictures: ScreenImages {
            before: before.as_ref().and_then(|f| encode(f, opts.format)),
            after: after.as_ref().and_then(|f| encode(f, opts.format)),
            regions: Vec::new(),
        },
    })
}

/// Cuts the drawn screen's crops from `remaining` (`None`: keep the areas, cut no crops)
/// and attaches the layers in each area.
fn finish_screen(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    pair: ScreenPair,
    readers: (ImageReader<'_>, ImageReader<'_>),
    opts: &RenderOpts,
    remaining: Option<&mut usize>,
    drawn: DrawnScreen,
) -> ScreenRender {
    let DrawnScreen {
        scale,
        width,
        height,
        mut capped,
        transforms,
        mask,
        mut regions,
        mut signatures,
        pixels,
        mut pictures,
    } = drawn;
    let mask: Vec<bool> = (0..width as usize * height as usize)
        .map(|i| mask.contains(i))
        .collect();
    if let (Some(a), Some(b), Some(remaining)) = (pair.old, pair.new, remaining) {
        let result = crate::render_crops::render_crops(
            renderers,
            docs,
            (a, b),
            readers,
            regions,
            (transforms, scale, opts.format, (width, height)),
            remaining,
        );
        let (kept, crops, crops_capped, indices) = result;
        regions = kept;
        pictures.regions = crops;
        capped |= crops_capped;
        let mut all = signatures.into_iter().map(Some).collect::<Vec<_>>();
        signatures = indices.iter().filter_map(|&i| all[i].take()).collect();
    }
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
    ScreenRender {
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
        pictures,
        signatures,
        transforms,
        mask,
        capped,
    }
}
#[cfg(test)]
mod tests {
    use super::can_redraw_root;

    #[test]
    fn only_edits_that_can_draw_count_as_root_edits() {
        for field in [
            "transform.m00",
            "transform.m01",
            "transform.m02",
            "transform.m10",
            "transform.m11",
            "transform.m12",
            "parentIndex.position",
            "name",
            "description",
            "exportSettings.0.suffix",
            "constraints.horizontal",
            "horizontalConstraint",
            "verticalConstraint",
        ] {
            assert!(!can_redraw_root(field), "{field}");
        }
        for field in [
            "fillPaints.0.transform.m02",
            "size.x",
            "opacity",
            "effects.0.offset.x",
        ] {
            assert!(can_redraw_root(field), "{field}");
        }
    }
}
