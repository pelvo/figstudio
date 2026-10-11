//! The layers a design context covers, in pre-order with tree levels, instances resolved.
use figstudio_fig::{
    Document, Guid, Val,
    resolve::{self, InstancePath, Walk, walk_instance, walk_path},
};
use std::borrow::Cow;
use std::sync::Arc;

/// Where a layer comes from: a document node or a node inside an instance.
#[derive(Clone, Debug)]
pub(super) enum Target {
    Node(Guid),
    Path(InstancePath),
}

impl Target {
    pub(super) fn id(&self) -> String {
        match self {
            Self::Node(id) => id.to_string(),
            Self::Path(path) => path.to_string(),
        }
    }
}

/// One layer with its effective (override-applied) data.
#[derive(Debug)]
pub(super) struct Item<'d> {
    /// Tree level below the requested root (the root is 0).
    pub(super) level: usize,
    pub(super) target: Target,
    pub(super) kind: &'d str,
    pub(super) data: Cow<'d, Val>,
}

/// The collected layers and what could not be collected.
#[derive(Debug, Default)]
pub(super) struct Layers<'d> {
    pub(super) items: Vec<Item<'d>>,
    /// More layers exist than `limit` allowed.
    pub(super) truncated: bool,
    /// Instances whose content could not be resolved (missing component, cycle, depth).
    pub(super) skipped_instances: usize,
}

/// Layers from `root` down, at most `limit`; hidden document nodes are left out with their
/// subtrees (hidden nodes inside instances are dropped while writing). `None` when the
/// instance path does not resolve.
pub(super) fn collect<'d>(doc: &'d Document, root: &Target, limit: usize) -> Option<Layers<'d>> {
    let definitions = Arc::new(resolve::definitions(doc));
    let mut layers = Layers::default();
    match root {
        Target::Path(path) => {
            let walk = walk_path(doc, &definitions, path, None, limit).ok()?;
            layers.add_walk(doc, walk, 0);
        }
        Target::Node(id) => {
            let mut stack = vec![(*id, 0)];
            while let Some((id, level)) = stack.pop() {
                let Some(node) = doc.node(id) else { continue };
                if level > 0 && node.data.flag("visible") == Some(false) {
                    continue;
                }
                if layers.items.len() >= limit {
                    layers.truncated = true;
                    break;
                }
                layers.items.push(Item {
                    level,
                    target: Target::Node(id),
                    kind: &node.kind,
                    data: Cow::Borrowed(&node.data),
                });
                if node.kind == "INSTANCE" {
                    let left = limit - layers.items.len();
                    match walk_instance(doc, &definitions, id, None, left) {
                        Ok(walk) => layers.add_walk(doc, walk, level),
                        Err(_) => layers.skipped_instances += 1,
                    }
                } else {
                    stack.extend(doc.children(id).iter().rev().map(|c| (*c, level + 1)));
                }
            }
        }
    }
    Some(layers)
}

impl<'d> Layers<'d> {
    fn add_walk(&mut self, doc: &'d Document, walk: Walk, base: usize) {
        self.truncated |= walk.truncated;
        self.skipped_instances += walk.skipped_instances;
        for node in walk.nodes {
            let kind = doc.node(node.node).map_or("UNKNOWN", |n| n.kind.as_str());
            self.items.push(Item {
                level: base + node.level,
                target: Target::Path(node.path),
                kind,
                data: Cow::Owned(node.data),
            });
        }
    }
}
