//! Bounds of what a node draws, and the size of its subtree, for the whole-node shadow guards.

use figstudio_fig::{Guid, Node, Val};
use vello_cpu::kurbo::{Affine, Rect};

use crate::geom::{GeomPath, affine, clips_content, size, transformed_bounds};
use crate::shadow_layer::{MAX_BOUNDS_VISITS, MAX_REPLAY_DESCENDANTS};
use crate::{Ctx, overhang, visible};

impl Ctx<'_> {
    /// Output bounds of everything the node draws: its box and ink, the stored geometry it
    /// actually draws (fill and stroke outlines, glyphs: the same blobs the draw decodes, boxed
    /// once per blob), and, unless it clips, its descendants. `fill_paths` are the node's own
    /// already decoded fill paths. Anything whose extent is not known cheaply (an instance in
    /// the subtree, overrides or scale in scope, text laid out from fonts, a vector with only a
    /// network, over `MAX_BOUNDS_VISITS` nodes) makes the whole target stand in, which is safe
    /// whatever the document holds because no layer can exceed the target (plus filter padding).
    pub(crate) fn alpha_bounds(
        &mut self,
        node: &Node,
        data: &Val,
        world: Affine,
        fill_paths: &[GeomPath],
    ) -> Rect {
        let (w, h) = size(data);
        let m = overhang(&node.kind, data, w, h);
        let own = transformed_bounds(world, Rect::new(-m, -m, w + m, h + m));
        let mut out = own;
        let mut grow = |r: Rect| out = out.union(r);
        for g in fill_paths {
            grow(transformed_bounds(
                world,
                g.path.control_box().inflate(m, m),
            ));
        }
        let Some(stroke) = self.stroke_box(data) else {
            return self.screen;
        };
        if let Some(r) = stroke {
            grow(transformed_bounds(world, r.inflate(m, m)));
        }
        // A clip keeps everything inside it, instances and overrides included.
        if clips_content(&node.kind, data) {
            return out;
        }
        if node.kind == "INSTANCE" || self.patches_in_scope() {
            return self.screen;
        }
        let mut stack: Vec<(Guid, Affine)> = self
            .doc
            .children(node.id)
            .iter()
            .map(|c| (*c, world))
            .collect();
        let mut visits = 0;
        while let Some((id, parent)) = stack.pop() {
            let Some(n) = self.doc.node(id) else {
                continue;
            };
            let d = &n.data;
            if !visible(d) {
                continue;
            }
            visits += 1;
            if n.kind == "INSTANCE" || visits > MAX_BOUNDS_VISITS || !self.charge_work() {
                return self.screen;
            }
            let world = parent * affine(d.get("transform"));
            let (w, h) = size(d);
            let m = overhang(&n.kind, d, w, h);
            grow(transformed_bounds(world, Rect::new(-m, -m, w + m, h + m)));
            let Some(own) = self.drawn_box(n, d) else {
                return self.screen;
            };
            if let Some(r) = own {
                grow(transformed_bounds(world, r.inflate(m, m)));
            }
            if !clips_content(&n.kind, d) {
                stack.extend(self.doc.children(id).iter().map(|c| (*c, world)));
            }
        }
        out
    }

    /// Box, in the node's own space, of the stored geometry a descendant draws; `Some(None)`
    /// when it stores none, `None` when its extent is unknown.
    fn drawn_box(&mut self, node: &Node, data: &Val) -> Option<Option<Rect>> {
        let mut out = self.stroke_box(data)?;
        let mut grow = |r: Option<Rect>| {
            if let Some(r) = r {
                out = Some(out.map_or(r, |o| o.union(r)));
            }
        };
        let fills = self.geometry_box(data, "fillGeometry")?;
        grow(fills);
        if node.kind == "TEXT" {
            grow(self.glyph_box(data)?);
        } else if node.kind == "VECTOR"
            && fills.is_none()
            && data.path(&["vectorData", "vectorNetworkBlob"]).is_some()
        {
            // Rebuilt from the editable network: not boxed here.
            return None;
        }
        Some(out)
    }

    /// Box of the node's stored stroke outlines (`Some(None)`: none stored).
    fn stroke_box(&mut self, data: &Val) -> Option<Option<Rect>> {
        self.geometry_box(data, "strokeGeometry")
    }

    /// Union of the boxes of the blobs listed under `key`; `None` when a listed blob cannot be read.
    fn geometry_box(&mut self, data: &Val, key: &str) -> Option<Option<Rect>> {
        let mut out: Option<Rect> = None;
        for item in data.get(key).map(Val::items).unwrap_or_default() {
            let index = item.get("commandsBlob").and_then(Val::as_u32)?;
            if let Some(r) = self.blob_box(index)? {
                out = Some(out.map_or(r, |o| o.union(r)));
            }
        }
        Some(out)
    }

    /// Box of the node's stored glyph outlines in node space; `None` when the text is laid out
    /// from system fonts (unknown extent) instead of stored glyphs.
    fn glyph_box(&mut self, data: &Val) -> Option<Option<Rect>> {
        let Some(glyphs) = data.path(&["derivedTextData", "glyphs"]) else {
            return (crate::text::characters(data).trim().is_empty()).then_some(None);
        };
        let mut out: Option<Rect> = None;
        for (i, glyph) in glyphs.items().iter().enumerate() {
            if i % 16 == 0 && !self.charge_work() {
                return None;
            }
            let index = glyph.get("commandsBlob").and_then(Val::as_u32)?;
            let Some(b) = self.blob_box(index)? else {
                continue;
            };
            let size = f64::from(glyph.num("fontSize").unwrap_or(12.0));
            let pos = glyph.get("position");
            let x = f64::from(pos.and_then(|p| p.num("x")).unwrap_or(0.0));
            let y = f64::from(pos.and_then(|p| p.num("y")).unwrap_or(0.0));
            // The same transform `text::stored_glyphs` applies: em units, y up.
            let at = Affine::translate((x, y)) * Affine::scale_non_uniform(size, -size);
            let r = transformed_bounds(at, b);
            out = Some(out.map_or(r, |o| o.union(r)));
        }
        Some(out)
    }

    /// Box of one path blob, decoded at most once per render (`Some(None)`: draws nothing,
    /// `None`: malformed or too large to trust).
    fn blob_box(&mut self, index: u32) -> Option<Option<Rect>> {
        if let Some(known) = self.blob_boxes.get(&index) {
            return *known;
        }
        let bytes = self.doc.blobs.get(usize::try_from(index).ok()?)?;
        let boxed = if bytes.is_empty() {
            Some(None)
        } else {
            crate::geom::blob_bounds(bytes).map(Some)
        };
        self.blob_boxes.insert(index, boxed);
        boxed
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
