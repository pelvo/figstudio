//! Bounded resolved-instance picking. Paths omit ordinary groups, so ascend via tree_path.

use figstudio_fig::{
    Document, Guid, Val,
    resolve::{self, InstancePath, Resolved},
};
use figstudio_render::{Renderer, geom, kurbo::Point};
use std::sync::Arc;

const BUDGET: usize = 2048;
const DEPTH: usize = 128;

pub fn parent(doc: &Document, path: &InstancePath) -> Option<InstancePath> {
    let tree = resolve::tree_path(doc, path).ok()?;
    let parent = *tree.get(tree.len().checked_sub(2)?)?;
    if parent == path.instance() {
        return None;
    }
    let mut keys = path.keys().to_vec();
    keys.pop();
    let node = doc.node(parent)?;
    if node.kind != "INSTANCE" {
        keys.push(node.override_key());
    }
    let mut ids = vec![path.instance()];
    ids.extend(keys);
    InstancePath::new(ids)
}

fn children(
    doc: &Document,
    definitions: &Arc<resolve::Definitions>,
    instance: Guid,
    path: Option<&InstancePath>,
    budget: usize,
) -> Vec<Resolved> {
    let walk = match path {
        Some(path) => resolve::walk_path(doc, definitions, path, Some(1), budget),
        None => resolve::walk_instance(doc, definitions, instance, Some(1), budget),
    };
    walk.map(|w| w.nodes.into_iter().filter(|n| n.level == 1).collect())
        .unwrap_or_default()
}

pub fn first_child(
    doc: &Document,
    instance: Guid,
    path: Option<&InstancePath>,
) -> Option<InstancePath> {
    children(
        doc,
        &Arc::new(resolve::definitions(doc)),
        instance,
        path,
        BUDGET,
    )
    .into_iter()
    .find(|n| n.data.flag("visible") != Some(false) && n.data.flag("mask") != Some(true))
    .map(|n| n.path)
}

fn reverse_order(doc: &Document, kind: &str, data: &Val) -> bool {
    // Instance children are painted in the definition's stacking order.
    let data = if kind == "INSTANCE" {
        resolve::symbol_id(data)
            .and_then(|id| doc.node(id))
            .map_or(data, |n| &n.data)
    } else {
        data
    };
    data.flag("stackReverseZIndex") == Some(true)
}

struct Hit<'a> {
    doc: &'a Document,
    renderer: Renderer,
    definitions: Arc<resolve::Definitions>,
    left: usize,
}

impl Hit<'_> {
    fn walk(
        &mut self,
        instance: Guid,
        parent: Option<&InstancePath>,
        point: Point,
        depth: usize,
        reverse: bool,
    ) -> Vec<InstancePath> {
        if self.left == 0 || depth >= DEPTH {
            return Vec::new();
        }
        let mut nodes = children(self.doc, &self.definitions, instance, parent, self.left);
        self.left = self.left.saturating_sub(nodes.len().max(1));
        if !reverse {
            nodes.reverse();
        }
        for resolved in nodes {
            if resolved.data.flag("visible") == Some(false)
                || resolved.data.flag("mask") == Some(true)
            {
                continue;
            }
            let Some(node) = self.doc.node(resolved.node) else {
                continue;
            };
            let Some(bounds) = self.renderer.path_bounds(self.doc, &resolved.path) else {
                continue;
            };
            let Some((tf, data)) = self.renderer.path_layout(self.doc, &resolved.path) else {
                continue;
            };
            if !tf.determinant().is_finite() || tf.determinant().abs() < f64::EPSILON {
                continue;
            }
            let local = tf.inverse() * point;
            let (w, h) = geom::size(&data);
            let inside = bounds.contains(point)
                && local.x >= 0.0
                && local.y >= 0.0
                && local.x <= w
                && local.y <= h;
            if inside || !geom::clips_content(&node.kind, &data) {
                let mut hit = self.walk(
                    instance,
                    Some(&resolved.path),
                    point,
                    depth + 1,
                    reverse_order(self.doc, &node.kind, &data),
                );
                if !hit.is_empty() {
                    if node.kind != "SECTION" {
                        hit.insert(0, resolved.path);
                    }
                    return hit;
                }
            }
            if inside && node.kind != "SECTION" {
                return vec![resolved.path];
            }
        }
        Vec::new()
    }
}

pub fn hit_chain(doc: &Document, instance: Guid, point: Point) -> Vec<InstancePath> {
    Hit {
        doc,
        renderer: Renderer::new(),
        definitions: Arc::new(resolve::definitions(doc)),
        left: BUDGET,
    }
    .walk(
        instance,
        None,
        point,
        0,
        doc.node(instance)
            .is_some_and(|n| reverse_order(doc, &n.kind, &n.data)),
    )
}

pub fn click(
    doc: &Document,
    chain: &[InstancePath],
    selected: Option<&InstancePath>,
    deep: bool,
    drill: bool,
) -> Option<InstancePath> {
    if deep {
        return chain.last().cloned();
    }
    if let Some(selected) = selected {
        if let Some(index) = chain.iter().position(|p| p == selected) {
            return chain
                .get(index + usize::from(drill))
                .or(Some(selected))
                .cloned();
        }
        let mut ancestor = parent(doc, selected);
        for _ in 0..DEPTH {
            let Some(path) = ancestor else { break };
            if let Some(index) = chain.iter().position(|p| *p == path) {
                return chain.get(index + 1).or(Some(&path)).cloned();
            }
            ancestor = parent(doc, &path);
        }
    }
    chain.first().cloned()
}
