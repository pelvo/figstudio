//! The changed component (SYMBOL) a screen region belongs to, so one component edit groups as
//! one shared change however each screen's changed area happens to be cut.
use crate::{Region, screen_bounds, screen_bounds::screen_box, shared::ComponentKey};
use figstudio_fig::{Document, Guid, resolve::symbol_id};
use figstudio_render::kurbo::{Affine, Rect};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub(crate) struct Masters<'a> {
    docs: (&'a Document, &'a Document),
    /// Every added, removed or changed node of the report, on any page.
    changed: HashSet<Guid>,
    memo: BTreeMap<Guid, bool>,
}

impl<'a> Masters<'a> {
    pub(crate) fn new(docs: (&'a Document, &'a Document), changed: HashSet<Guid>) -> Self {
        Self {
            docs,
            changed,
            memo: BTreeMap::new(),
        }
    }

    pub(crate) fn docs(&self) -> (&'a Document, &'a Document) {
        self.docs
    }

    /// Variants of one component set group by the set; any other component by itself.
    fn group_of(&self, symbol: Guid) -> Guid {
        [self.docs.1, self.docs.0]
            .iter()
            .find_map(|d| d.node(symbol))
            .and_then(|n| n.parent)
            .filter(|p| {
                [self.docs.1, self.docs.0]
                    .iter()
                    .any(|d| d.node(*p).is_some_and(is_component_set))
            })
            .unwrap_or(symbol)
    }

    fn is_symbol(&self, id: Guid) -> bool {
        [self.docs.0, self.docs.1]
            .iter()
            .any(|d| d.node(id).is_some_and(|n| n.kind == "SYMBOL"))
    }

    /// A component changed when its subtree (either side) holds a changed node, including its
    /// own version fields, or holds an instance of another changed component.
    fn is_changed(&mut self, symbol: Guid) -> bool {
        if let Some(known) = self.memo.get(&symbol) {
            return *known;
        }
        self.memo.insert(symbol, false); // a cycle ends here
        let mut found = false;
        if self.is_symbol(symbol) {
            for doc in [self.docs.0, self.docs.1] {
                for id in screen_bounds::subtree(doc, Some(symbol)) {
                    if self.changed.contains(&id) {
                        found = true;
                    } else if let Some(inner) = doc
                        .node(id)
                        .filter(|n| n.kind == "INSTANCE")
                        .and_then(|n| symbol_id(&n.data))
                        .filter(|inner| *inner != symbol)
                    {
                        found = found || self.is_changed(inner);
                    }
                }
            }
        }
        self.memo.insert(symbol, found);
        found
    }

    /// Per region: the group and master of the smallest changed-master instance holding at least half
    /// of the region's changed pixels (a nested instance beats its container by being smaller).
    pub(crate) fn keys(
        &mut self,
        roots: (Option<Guid>, Option<Guid>),
        transforms: (Affine, Affine),
        regions: &[Region],
        mask: (&[bool], usize),
    ) -> Vec<Option<ComponentKey>> {
        let mut found: Vec<(Rect, Guid)> = Vec::new();
        for (doc, root, t) in [
            (self.docs.0, roots.0, transforms.0),
            (self.docs.1, roots.1, transforms.1),
        ] {
            for id in screen_bounds::subtree(doc, root) {
                let Some(node) = doc.node(id).filter(|n| n.kind == "INSTANCE") else {
                    continue;
                };
                if let Some(symbol) = symbol_id(&node.data).filter(|s| self.is_changed(*s))
                    && let Some(rect) = screen_box(doc, id, t)
                {
                    found.push((rect, symbol));
                }
            }
        }
        found.sort_by(|a, b| {
            a.0.area()
                .total_cmp(&b.0.area())
                .then_with(|| a.1.cmp(&b.1))
        });
        regions
            .iter()
            .map(|r| {
                let inside = |rect: Rect| changed_in(mask, r, Some(rect));
                let total = changed_in(mask, r, None);
                found
                    .iter()
                    .find(|(rect, _)| total > 0 && inside(*rect) * 2 >= total)
                    .map(|(_, master)| ComponentKey {
                        group: self.group_of(*master),
                        master: *master,
                    })
            })
            .collect()
    }
}

/// Decoded .fig files carry a component set as a FRAME with `isStateGroup`; the schema has no
/// separate kind, but a `COMPONENT_SET` string is accepted too.
fn is_component_set(node: &figstudio_fig::Node) -> bool {
    node.kind == "COMPONENT_SET" || node.data.flag("isStateGroup") == Some(true)
}

/// Changed pixels inside the region's box, optionally only those within `rect`.
fn changed_in((mask, width): (&[bool], usize), r: &Region, rect: Option<Rect>) -> usize {
    let (x0, y0) = (r.x as usize, r.y as usize);
    let (mut x1, mut y1) = (x0 + r.width as usize, y0 + r.height as usize);
    let (mut lo_x, mut lo_y) = (x0, y0);
    if let Some(rect) = rect {
        // A pixel counts when its centre is inside the rect.
        let clamp = |v: f64| v.max(0.0).min(usize::MAX as f64) as usize;
        lo_x = lo_x.max(clamp((rect.x0 - 0.5).ceil()));
        lo_y = lo_y.max(clamp((rect.y0 - 0.5).ceil()));
        x1 = x1.min(clamp((rect.x1 - 0.5).ceil()));
        y1 = y1.min(clamp((rect.y1 - 0.5).ceil()));
    }
    (lo_y..y1)
        .map(|y| {
            (lo_x..x1)
                .filter(|x| mask.get(y * width + x).copied().unwrap_or(false))
                .count()
        })
        .sum()
}
