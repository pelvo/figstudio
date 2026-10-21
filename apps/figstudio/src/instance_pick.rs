//! Bounded resolved-instance picking. Paths omit ordinary groups, so ascend via tree_path.

use figstudio_fig::{
    Document, Guid,
    resolve::{self, InstancePath},
};
use figstudio_render::{Renderer, geom, kurbo::Point};
use std::sync::Arc;

const BUDGET: usize = 2048;
const DEPTH: usize = 128;

pub fn parent(
    doc: &Document,
    definitions: &Arc<resolve::Definitions>,
    path: &InstancePath,
) -> Option<InstancePath> {
    let tree = resolve::tree_path_with(doc, definitions, path).ok()?;
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

pub fn first_child(
    doc: &Document,
    renderer: &mut Renderer,
    instance: Guid,
    path: Option<&InstancePath>,
) -> Option<InstancePath> {
    renderer
        .child_layouts(doc, instance, path, BUDGET)
        .into_iter()
        .find(|n| {
            doc.node(n.node).is_some_and(|node| node.kind != "SECTION")
                && n.data.flag("visible") != Some(false)
                && n.data.flag("mask") != Some(true)
        })
        .map(|n| n.path)
}

struct Hit<'a> {
    doc: &'a Document,
    renderer: &'a mut Renderer,
    left: usize,
}

impl Hit<'_> {
    fn walk(
        &mut self,
        instance: Guid,
        parent: Option<&InstancePath>,
        point: Point,
        depth: usize,
    ) -> Vec<InstancePath> {
        if self.left == 0 || depth >= DEPTH {
            return Vec::new();
        }
        let nodes = self
            .renderer
            .child_layouts(self.doc, instance, parent, self.left);
        for resolved in nodes {
            if self.left == 0 {
                break;
            }
            self.left -= 1;
            if resolved.data.flag("visible") == Some(false)
                || resolved.data.flag("mask") == Some(true)
            {
                continue;
            }
            let Some(node) = self.doc.node(resolved.node) else {
                continue;
            };
            let (bounds, tf, data) = (resolved.bounds, resolved.transform, resolved.data);
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
                let mut hit = self.walk(instance, Some(&resolved.path), point, depth + 1);
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

#[cfg(test)]
pub fn hit_chain(doc: &Document, instance: Guid, point: Point) -> Vec<InstancePath> {
    hit_chain_with(doc, &mut Renderer::new(), instance, point)
}

pub fn hit_chain_with(
    doc: &Document,
    renderer: &mut Renderer,
    instance: Guid,
    point: Point,
) -> Vec<InstancePath> {
    Hit {
        doc,
        renderer,
        left: BUDGET,
    }
    .walk(instance, None, point, 0)
}

pub fn click(
    doc: &Document,
    definitions: &Arc<resolve::Definitions>,
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
        let mut ancestor = parent(doc, definitions, selected);
        for _ in 0..DEPTH {
            let Some(path) = ancestor else { break };
            if let Some(index) = chain.iter().position(|p| *p == path) {
                return chain.get(index + 1).or(Some(&path)).cloned();
            }
            ancestor = parent(doc, definitions, &path);
        }
    }
    chain.first().cloned()
}
