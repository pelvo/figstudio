//! Standalone screen comparison; canvas placement never enters the pixel mask.
use crate::screen_limits::PageTally;
use crate::shared::{ComponentKey, RegionKey};
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

const SCREEN_PIXELS: usize = 4_194_304;

pub(crate) struct ScreenRender {
    pub(crate) screen: ScreenDiff,
    pub(crate) pictures: ScreenImages,
    signatures: Vec<crate::shared::Signature>,
    /// Per area, when the screen was drawn with [`Owners`]: the leaf layers that own its
    /// changed pixels and the changed component it belongs to.
    owners: Vec<(Vec<Guid>, Option<ComponentKey>)>,
    capped: bool,
}

/// A page screen's members and the page's components: what [`draw_screen`] needs to find,
/// while it still holds the changed-pixel mask, which layers and component own each area.
type Owners<'m, 'a> = (&'m [Guid], &'m mut Masters<'a>);

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
    let mut uncropped = 0;
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
                (false, Some((&members[index][..], &mut *masters))),
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
        uncropped += usize::from(full);
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
            owners,
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
        let mut components = Vec::with_capacity(owners.len());
        for (region, (owners, component)) in screen.regions.iter_mut().zip(owners) {
            // Layers mostly inside the area, plus the leaf layers that own its changed pixels
            // (a small change inside a wider layer), in member order.
            region.nodes = nodes
                .iter()
                .copied()
                .filter(|id| region.nodes.contains(id) || owners.contains(id))
                .collect();
            components.push(component);
        }
        screen.nodes = nodes;
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
    if uncropped > 0 {
        // Their areas may still fold into a shared card with another screen's crop, so the
        // note says only what was left out.
        notes.push(format!(
            "page {} picture byte limit reached; crops left out for {}",
            page.id,
            crate::html::plural(uncropped, "screen", "screens")
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
    /// Changed areas, before crops are cut; `signatures` has one per area, and so has
    /// `owners` when the screen was drawn with [`Owners`]. The mask itself is not kept
    /// between the two passes.
    regions: Vec<crate::Region>,
    signatures: Vec<crate::shared::Signature>,
    owners: Vec<(Vec<Guid>, Option<ComponentKey>)>,
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
    let drawn = draw_screen(
        (&mut *old, &mut *new),
        docs,
        pair,
        readers,
        opts,
        (merge, None),
    )?;
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
/// and the mask are dropped once the pictures are encoded and the areas' signatures (and,
/// with [`Owners`], their owners) taken. `merge` folds all areas into one box.
fn draw_screen(
    renderers: (&mut Renderer, &mut Renderer),
    docs: (&Document, &Document),
    pair: ScreenPair,
    readers: (ImageReader<'_>, ImageReader<'_>),
    opts: &RenderOpts,
    (merge, annotate): (bool, Option<Owners<'_, '_>>),
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
    let owners = annotate.map_or_else(Vec::new, |(members, masters)| {
        let mask = (&mask[..], width as usize);
        let components = masters.keys((pair.old, pair.new), transforms, &regions, mask);
        regions
            .iter()
            .zip(components)
            .map(|(region, component)| {
                let leaves = screen_bounds::containing_leaves(
                    region,
                    members,
                    docs,
                    pair,
                    (transforms, mask),
                );
                (leaves, component)
            })
            .collect()
    });
    Some(DrawnScreen {
        scale,
        width,
        height,
        capped,
        transforms,
        regions,
        signatures,
        pixels,
        owners,
        pictures: ScreenImages {
            before: before.as_ref().and_then(|f| encode(f, opts.format)),
            after: after.as_ref().and_then(|f| encode(f, opts.format)),
            regions: Vec::new(),
        },
    })
}

/// Cuts the drawn screen's crops from `remaining` (`None`: keep the areas, cut no crops)
/// and attaches the layers in each area. An area whose crop could not be cut is dropped,
/// so the screen ends with one crop per area, in order, or none.
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
        mut regions,
        mut signatures,
        mut owners,
        pixels,
        mut pictures,
    } = drawn;
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
        if !owners.is_empty() {
            let mut all = owners.into_iter().map(Some).collect::<Vec<_>>();
            owners = indices.iter().filter_map(|&i| all[i].take()).collect();
        }
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
        owners,
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
