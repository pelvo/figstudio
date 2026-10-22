//! Subtree membership and screen-local boxes use the renderer's transforms.
use crate::{PageDiff, Region, screens::ScreenPair};
use figstudio_fig::{Document, Guid};
use figstudio_render::{
    absolute_transform,
    geom::size,
    kurbo::{Affine, Rect},
};
use std::collections::BTreeSet;

pub(crate) fn subtree(doc: &Document, root: Option<Guid>) -> BTreeSet<Guid> {
    let mut found = BTreeSet::new();
    let mut stack: Vec<_> = root.into_iter().collect();
    while let Some(id) = stack.pop() {
        if found.insert(id) {
            stack.extend(doc.children(id));
        }
    }
    found
}

pub(crate) fn members(
    page: &PageDiff,
    docs: (&Document, &Document),
    pair: ScreenPair,
) -> Vec<Guid> {
    let a = subtree(docs.0, pair.old);
    let b = subtree(docs.1, pair.new);
    page.added
        .iter()
        .map(|n| n.id)
        .filter(|id| b.contains(id))
        .chain(
            page.removed
                .iter()
                .map(|n| n.id)
                .filter(|id| a.contains(id)),
        )
        .chain(
            page.changed
                .iter()
                .map(|n| n.id)
                .filter(|id| a.contains(id) || b.contains(id)),
        )
        .collect()
}

pub(crate) fn screen_box(doc: &Document, id: Guid, transform: Affine) -> Option<Rect> {
    let (w, h) = size(&doc.node(id)?.data);
    let rect =
        (transform * absolute_transform(doc, id)).transform_rect_bbox(Rect::new(0.0, 0.0, w, h));
    [rect.x0, rect.y0, rect.x1, rect.y1]
        .iter()
        .all(|n| n.is_finite())
        .then_some(rect)
}

/// Changed leaf layers (no children, not the screen roots) whose box holds the centre of
/// `region`: a small change inside a wide layer, such as one new glyph of a long text line,
/// which the half-of-the-layer rule in [`region_nodes`] cannot match.
pub(crate) fn containing_leaves(
    region: &Region,
    changed: &[Guid],
    docs: (&Document, &Document),
    pair: ScreenPair,
    transforms: (Affine, Affine),
) -> Vec<Guid> {
    let area = Rect::new(
        region.x as f64,
        region.y as f64,
        (region.x + region.width) as f64,
        (region.y + region.height) as f64,
    );
    let sides = [
        (docs.0, subtree(docs.0, pair.old), transforms.0),
        (docs.1, subtree(docs.1, pair.new), transforms.1),
    ];
    changed
        .iter()
        .copied()
        .filter(|id| Some(*id) != pair.old && Some(*id) != pair.new)
        .filter(|id| {
            sides.iter().any(|(doc, members, t)| {
                members.contains(id)
                    && doc.children(*id).is_empty()
                    && screen_box(doc, *id, *t).is_some_and(|r| r.contains(area.center()))
            })
        })
        .collect()
}

pub(crate) fn region_nodes(
    regions: &mut [Region],
    ids: &[Guid],
    docs: (&Document, &Document),
    pair: ScreenPair,
    transforms: (Affine, Affine),
    size: (u32, u32),
) {
    let a = subtree(docs.0, pair.old);
    let b = subtree(docs.1, pair.new);
    let canvas = Rect::new(0.0, 0.0, size.0 as f64, size.1 as f64);
    for region in regions {
        let area = Rect::new(
            region.x as f64,
            region.y as f64,
            (region.x + region.width) as f64,
            (region.y + region.height) as f64,
        );
        region.nodes = ids
            .iter()
            .copied()
            .filter(|id| {
                [(docs.0, &a, transforms.0), (docs.1, &b, transforms.1)]
                    .into_iter()
                    .any(|(doc, members, t)| {
                        members.contains(id)
                            && screen_box(doc, *id, t).is_some_and(|r| {
                                let r = r.intersect(canvas);
                                r.area() > 0.0 && r.intersect(area).area() >= r.area() * 0.5
                            })
                    })
            })
            .collect();
    }
}
