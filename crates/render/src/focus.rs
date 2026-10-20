//! Drawing one node inside an instance on its own, laid out exactly as the renderer draws it
//! as part of the instance (overrides, uniform scale and constraints included).
use std::borrow::Cow;

use figstudio_fig::resolve::{self, InstancePath};
use figstudio_fig::{Guid, Val};
use vello_cpu::kurbo::Affine;

use crate::geom::{affine, size};
use crate::{Ctx, absolute_transform_with_budget, visible};

impl Ctx<'_> {
    /// Walks from the instance down to the node at `path` the way `draw_instance` and
    /// `draw_children` would, then draws only that node when `draw` is set. Returns the node's
    /// captured layout under `transform`, or `None` when the path does not
    /// resolve or the node or one of its ancestors is hidden.
    pub(crate) fn focus<T>(
        &mut self,
        path: &InstancePath,
        transform: Affine,
        draw: bool,
        capture: impl FnOnce(Affine, &Val, bool) -> T,
    ) -> Option<T> {
        let doc = self.doc;
        let chain = resolve::tree_path(doc, path).ok()?;
        let (top, rest) = chain.split_first()?;
        let mut parent = doc.node(*top)?;
        let mut parent_data: Cow<'_, Val> = Cow::Borrowed(&parent.data);
        let mut world = match parent.parent {
            Some(p) => transform * absolute_transform_with_budget(doc, p, &mut self.work_left)?,
            None => transform,
        };
        // Override path of `parent`; the outermost instance is drawn without one.
        let mut parent_path: Vec<Guid> = Vec::new();
        for (index, id) in rest.iter().enumerate() {
            if !visible(&parent_data) {
                return None;
            }
            world *= affine(parent_data.get("transform"));
            let node = doc.node(*id)?;
            let (base, constraint) = if parent.kind == "INSTANCE" {
                let symbol = resolve::symbol_id(&parent_data).and_then(|s| doc.node(s))?;
                if !self.overrides.can_enter(symbol.id) {
                    return None;
                }
                self.overrides.enter(symbol, &parent_data, &parent_path);
                (parent_path.clone(), None)
            } else {
                // As in `draw_children`: frames are not part of override paths, and a resized
                // frame moves its children by their constraints.
                let base = parent_path[..parent_path.len().saturating_sub(1)].to_vec();
                let resized = size(&parent_data) != size(&parent.data);
                (
                    base,
                    resized.then_some((&parent.data, parent_data.as_ref())),
                )
            };
            let mut node_path = base;
            node_path.push(node.override_key());
            let (data, stale, ink) = self.patched(node, Some(&node_path), constraint);
            if index + 1 == rest.len() {
                if !visible(&data) {
                    return None;
                }
                let transform = world * affine(data.get("transform"));
                let captured = capture(transform, &data, stale);
                if draw {
                    self.draw_with_data(node, world, Some(&node_path), Some((data, stale, ink)));
                }
                return Some(captured);
            }
            parent = node;
            parent_data = Cow::Owned(data.into_owned());
            parent_path = node_path;
        }
        None
    }
}
