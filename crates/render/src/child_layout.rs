//! Read-only child layout, sharing the drawing context's override and constraint machinery.
use std::{borrow::Cow, sync::Arc};

use figstudio_fig::{
    Document, Guid, Node, Val,
    resolve::{self, InstancePath},
};
use vello_cpu::{
    RenderContext, RenderSettings,
    kurbo::{Affine, Rect},
};

use crate::{
    Ctx, MAX_RENDER_WORK, Renderer,
    geom::{affine, size, transformed_bounds},
    variables,
};

/// Cumulative inspection work, useful for checking cache reuse without timing assertions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InspectionWork {
    pub definitions_builds: usize,
    pub focus_walks: usize,
}

/// One immediate child's resolved layout. Bounds are its page-space layout box, not ink or
/// subtree bounds: unclipped children can draw outside it.
#[derive(Clone, Debug)]
pub struct ChildLayout {
    pub node: Guid,
    pub path: InstancePath,
    pub transform: Affine,
    pub data: Val,
    pub bounds: Rect,
}

impl Renderer {
    /// Definitions shared with the renderer's per-document indexes.
    pub fn definitions(&mut self, doc: &Document) -> Arc<resolve::Definitions> {
        self.document_indexes(doc).0
    }

    pub fn inspection_work(&self) -> InspectionWork {
        self.inspection_work
    }

    pub(super) fn document_indexes(
        &mut self,
        doc: &Document,
    ) -> (Arc<resolve::Definitions>, Arc<variables::Variables>) {
        if let Some((instance, d, v)) = &self.indexes
            && *instance == doc.instance
        {
            return (d.clone(), v.clone());
        }
        self.inspection_work.definitions_builds += 1;
        let definitions = Arc::new(resolve::definitions(doc));
        let variables = Arc::new(variables::Variables::new(doc));
        self.indexes = Some((doc.instance, definitions.clone(), variables.clone()));
        (definitions, variables)
    }

    /// Focus once at `parent` (or the outer instance), then lay out at most `max_nodes`
    /// immediate children. In document order; consumers choose their own stacking direction.
    /// Hidden parents, unresolved paths and expansion cycles return no children. Hidden
    /// children are included so a caller's traversal budget counts them too.
    pub fn child_layouts(
        &mut self,
        doc: &Document,
        instance: Guid,
        parent: Option<&InstancePath>,
        max_nodes: usize,
    ) -> Vec<ChildLayout> {
        if max_nodes == 0
            || parent.is_some_and(|p| p.instance() != instance)
            || !doc.node(instance).is_some_and(|n| n.kind == "INSTANCE")
        {
            return Vec::new();
        }
        let mut canvas = RenderContext::new_with(1, 1, RenderSettings::default());
        let mut ctx = self.context(doc, &mut canvas, Rect::ZERO, MAX_RENDER_WORK, 0);
        ctx.child_layouts(instance, parent, max_nodes.min(MAX_RENDER_WORK))
    }
}

impl Ctx<'_> {
    /// The same override path and resized-parent constraint correction for drawing and picking.
    pub(super) fn child_patch<'n>(
        &self,
        node: &Node,
        data: &Val,
        child: &'n Node,
        path: Option<&[Guid]>,
    ) -> (Option<Vec<Guid>>, (Cow<'n, Val>, bool, bool)) {
        let child_path = path.map(|p| {
            let mut v = p.to_vec();
            v.push(child.override_key());
            v
        });
        let constraint =
            (path.is_some() && size(data) != size(&node.data)).then_some((&node.data, data));
        let patched = self.patched(child, child_path.as_deref(), constraint);
        (child_path, patched)
    }

    fn child_layouts(
        &mut self,
        instance: Guid,
        parent: Option<&InstancePath>,
        max_nodes: usize,
    ) -> Vec<ChildLayout> {
        let Some(focus) = self.focus_node(instance, parent, Affine::IDENTITY) else {
            return Vec::new();
        };
        let world = focus.parent * affine(focus.data.get("transform"));
        let (node, data, base) = if focus.node.kind == "INSTANCE" {
            let Some(symbol) = resolve::symbol_id(&focus.data).and_then(|id| self.doc.node(id))
            else {
                return Vec::new();
            };
            if !self.overrides.can_enter(symbol.id) {
                return Vec::new();
            }
            self.overrides.enter(symbol, &focus.data, &focus.path);
            (symbol, Cow::Borrowed(&symbol.data), focus.path)
        } else {
            let base = focus.path[..focus.path.len().saturating_sub(1)].to_vec();
            (focus.node, focus.data, base)
        };
        let mut layouts = Vec::new();
        for id in self.doc.children(node.id).iter().take(max_nodes) {
            if !self.charge_work() {
                break;
            }
            let Some(child) = self.doc.node(*id) else {
                continue;
            };
            let (path, (data, stale, _)) = self.child_patch(node, &data, child, Some(&base));
            let mut keys = vec![instance];
            keys.extend(path.into_iter().flatten());
            let Some(path) = InstancePath::new(keys) else {
                continue;
            };
            let transform = world * affine(data.get("transform"));
            let (w, h) = size(&data);
            let bounds = transformed_bounds(transform, Rect::new(0.0, 0.0, w, h));
            let mut data = data.into_owned();
            if stale && let Val::Obj(fields) = &mut data {
                fields.remove("derivedTextData");
            }
            layouts.push(ChildLayout {
                node: child.id,
                path,
                transform,
                data,
                bounds,
            });
        }
        layouts
    }
}
