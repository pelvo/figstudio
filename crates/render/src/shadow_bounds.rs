//! Bounds of what a node draws, and the size of its subtree, for the whole-node shadow guards.

use figstudio_fig::{Guid, Node, Val};
use vello_cpu::kurbo::{Affine, Rect};

use crate::geom::{affine, clips_content, size, transformed_bounds};
use crate::shadow_layer::{MAX_BOUNDS_VISITS, MAX_REPLAY_DESCENDANTS};
use crate::{Ctx, overhang, visible};

impl Ctx<'_> {
    /// Output bounds of everything the node draws: its own box and ink (strokes, effects;
    /// never trimmed, a clip only trims what is inside it), and, unless it clips, its
    /// descendants. A subtree with an instance in it (overrides, swaps, derived data and
    /// resizing can all change what it draws), a node that an enclosing instance's overrides
    /// apply to, or over `MAX_BOUNDS_VISITS` nodes is not bounded: the whole target stands in. That is safe whatever the overrides are, because
    /// no layer can be larger than the target, and the aggregate layer budget caps the rest.
    pub(crate) fn alpha_bounds(&mut self, node: &Node, data: &Val, world: Affine) -> Rect {
        let (w, h) = size(data);
        let m = overhang(&node.kind, data, w, h);
        let own = transformed_bounds(world, Rect::new(-m, -m, w + m, h + m));
        if clips_content(&node.kind, data) {
            return own;
        }
        if self.patches_in_scope() {
            return self.screen;
        }
        let mut out = own;
        let mut stack = vec![(node.id, world, true)];
        let mut visits = 0;
        while let Some((id, world, root)) = stack.pop() {
            let Some(n) = self.doc.node(id) else {
                continue;
            };
            let d = if root { data } else { &n.data };
            if !root && !visible(d) {
                continue;
            }
            if n.kind == "INSTANCE" {
                return self.screen;
            }
            // The node itself is already charged by the caller.
            if !root {
                visits += 1;
                if visits > MAX_BOUNDS_VISITS || !self.charge_work() {
                    return self.screen;
                }
            }
            let world = if root {
                world
            } else {
                let world = world * affine(d.get("transform"));
                let (w, h) = size(d);
                let m = overhang(&n.kind, d, w, h);
                out = out.union(transformed_bounds(world, Rect::new(-m, -m, w + m, h + m)));
                world
            };
            if !root && clips_content(&n.kind, d) {
                continue;
            }
            stack.extend(self.doc.children(id).iter().map(|c| (*c, world, false)));
        }
        out
    }

    /// Whether an enclosing instance's overrides, derived data or scale apply to this node.
    fn patches_in_scope(&self) -> bool {
        !self.patches.is_empty() || self.instance_scales.iter().any(|s| (s - 1.0).abs() > 1e-9)
    }

    /// The nodes drawn inside `node`: its children, or its symbol's children for an instance.
    pub(crate) fn drawn_children(&self, node: &Node) -> impl Iterator<Item = Guid> + '_ {
        let symbol = (node.kind == "INSTANCE")
            .then(|| symbol_of(&node.data))
            .flatten();
        self.doc
            .children(node.id)
            .iter()
            .chain(symbol.map_or(&[][..], |s| self.doc.children(s)))
            .copied()
    }

    /// Whether the node's subtree is small enough to be worth redrawing (charged to the work
    /// budget; counts a symbol's nodes for instances) and returns how many nodes it has. A cheap
    /// hint: the runtime floor in `draw_behind_shadows` is what bounds the redraw work.
    pub(crate) fn small_subtree(&mut self, node: &Node) -> Option<usize> {
        let mut stack = vec![node.id];
        let mut count = 0;
        while let Some(id) = stack.pop() {
            count += 1;
            // The node itself is already charged by the caller.
            if count > MAX_REPLAY_DESCENDANTS || (count > 1 && !self.charge_work()) {
                return None;
            }
            if let Some(n) = self.doc.node(id) {
                stack.extend(self.drawn_children(n));
            }
        }
        Some(count)
    }
}

/// The symbol an instance draws.
fn symbol_of(data: &Val) -> Option<Guid> {
    data.get("overriddenSymbolID")
        .and_then(Val::as_guid)
        .or_else(|| {
            data.path(&["symbolData", "symbolID"])
                .and_then(Val::as_guid)
        })
}
