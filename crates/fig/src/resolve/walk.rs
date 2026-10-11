//! Walks an instance's resolved children and looks up one child by its Figma-style id path.
use std::sync::Arc;

use super::types::Kind;
pub use super::types::{InstancePath, ResolveError};
use super::{Definitions, Overrides, definitions, symbol_id};
use crate::{Document, Guid, Node, Val};

/// One node inside an instance with every override applied.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// Figma-style id of this node.
    pub path: InstancePath,
    /// The component node this one is drawn from.
    pub node: Guid,
    /// The component node's data with overrides, properties and swaps applied.
    pub data: Val,
    /// Tree depth below where the walk started: an instance's children are level 1, the
    /// target of [`walk_path`] or [`resolve_path`] is level 0. Every tree level counts.
    pub level: usize,
}

/// The resolved children of one instance, in document (back to front) pre-order.
#[derive(Clone, Debug, Default)]
pub struct Walk {
    pub nodes: Vec<Resolved>,
    /// The node cap stopped the walk before every node was listed.
    pub truncated: bool,
    /// Nested instances not expanded: missing component, cycle or [`super::MAX_INSTANCE_DEPTH`].
    pub skipped_instances: usize,
}

/// One step of the walk's explicit stack (no recursion, so deep files cannot overflow it).
/// Children are queued one at a time, so a huge component costs nothing past the node cap.
#[derive(Debug)]
enum Step {
    Visit(Guid, Vec<Guid>, usize),
    Children(Guid, Vec<Guid>, usize, usize),
    Leave,
}

/// Default node cap of [`resolve_instance`]; bounds work on hostile, exponentially nested files.
pub const MAX_RESOLVED_NODES: usize = 100_000;

/// Every node inside `instance`, at most [`MAX_RESOLVED_NODES`].
pub fn resolve_instance(doc: &Document, instance: Guid) -> Result<Walk, ResolveError> {
    let definitions = Arc::new(definitions(doc));
    walk_instance(doc, &definitions, instance, None, MAX_RESOLVED_NODES)
}

/// Nodes inside `instance` down to `max_level` (unbounded when `None`; its children are level
/// 1, every tree level counts), at most `max_nodes`. `definitions` is [`definitions`]`(doc)`,
/// built once when many instances are walked.
pub fn walk_instance(
    doc: &Document,
    definitions: &Arc<Definitions>,
    instance: Guid,
    max_level: Option<usize>,
    max_nodes: usize,
) -> Result<Walk, ResolveError> {
    let node = doc
        .node(instance)
        .ok_or_else(|| ResolveError::new(Kind::UnknownNode))?;
    if node.kind != "INSTANCE" {
        return Err(ResolveError::new(Kind::NotInstance));
    }
    let mut overrides = Overrides::new(definitions.clone());
    let mut stack = Vec::new();
    if max_level.is_none_or(|limit| limit >= 1) {
        enter(doc, &mut overrides, &node.data, Vec::new(), 1, &mut stack)?;
    }
    Ok(run(doc, overrides, instance, stack, max_level, max_nodes))
}

/// The node at `path` (level 0) and its descendants down to `max_level`, at most `max_nodes`.
/// The node is found by its position in the tree, so ordinary frames between the nested
/// instances of the path do not count against `max_level`.
pub fn walk_path(
    doc: &Document,
    definitions: &Arc<Definitions>,
    path: &InstancePath,
    max_level: Option<usize>,
    max_nodes: usize,
) -> Result<Walk, ResolveError> {
    let mut overrides = Overrides::new(definitions.clone());
    let (_, node, _) = descend(doc, &mut overrides, path)?;
    let base = path.keys()[..path.keys().len() - 1].to_vec();
    let stack = vec![Step::Visit(node.id, base, 0)];
    Ok(run(
        doc,
        overrides,
        path.instance(),
        stack,
        max_level,
        max_nodes,
    ))
}

fn run(
    doc: &Document,
    mut overrides: Overrides,
    instance: Guid,
    mut stack: Vec<Step>,
    max_level: Option<usize>,
    max_nodes: usize,
) -> Walk {
    let mut walk = Walk::default();
    while let Some(step) = stack.pop() {
        #[cfg(test)]
        tests::PEAK.with(|peak| peak.set(peak.get().max(stack.len())));
        match step {
            Step::Leave => overrides.leave(),
            Step::Children(parent, base, level, index) => {
                if let Some(child) = doc.children(parent).get(index) {
                    stack.push(Step::Children(parent, base.clone(), level, index + 1));
                    stack.push(Step::Visit(*child, base, level));
                }
            }
            Step::Visit(id, base, level) => {
                let Some(child) = doc.node(id) else {
                    continue;
                };
                if walk.nodes.len() >= max_nodes {
                    walk.truncated = true;
                    break;
                }
                let mut path = base.clone();
                path.push(child.override_key());
                let data = overrides.resolve(doc, child, &path).into_owned();
                let deeper = max_level.is_none_or(|limit| level < limit);
                if deeper && child.kind == "INSTANCE" {
                    let entered = enter(
                        doc,
                        &mut overrides,
                        &data,
                        path.clone(),
                        level + 1,
                        &mut stack,
                    );
                    walk.skipped_instances += usize::from(entered.is_err());
                } else if deeper {
                    stack.push(Step::Children(id, base, level + 1, 0));
                }
                let mut guids = vec![instance];
                guids.extend(path);
                walk.nodes.push(Resolved {
                    path: InstancePath(guids),
                    node: id,
                    data,
                    level,
                });
            }
        }
    }
    walk
}

/// Enters the instance with resolved `data` at override path `base` and queues its children.
fn enter(
    doc: &Document,
    overrides: &mut Overrides,
    data: &Val,
    base: Vec<Guid>,
    level: usize,
    stack: &mut Vec<Step>,
) -> Result<(), ResolveError> {
    let symbol = enter_symbol(doc, overrides, data, &base)?;
    stack.push(Step::Leave);
    stack.push(Step::Children(symbol.id, base, level, 0));
    Ok(())
}

fn enter_symbol<'d>(
    doc: &'d Document,
    overrides: &mut Overrides,
    data: &Val,
    base: &[Guid],
) -> Result<&'d Node, ResolveError> {
    let symbol = symbol_id(data)
        .and_then(|id| doc.node(id))
        .ok_or_else(|| ResolveError::new(Kind::MissingComponent))?;
    if !overrides.can_enter(symbol.id) {
        return Err(ResolveError::new(Kind::TooDeep));
    }
    overrides.enter(symbol, data, base);
    Ok(symbol)
}

/// The node at `path` with every override applied.
pub fn resolve_path(doc: &Document, path: &InstancePath) -> Result<Resolved, ResolveError> {
    let mut overrides = Overrides::new(Arc::new(definitions(doc)));
    let (_, node, data) = descend(doc, &mut overrides, path)?;
    Ok(Resolved {
        path: path.clone(),
        node: node.id,
        data,
        level: 0,
    })
}

/// The document nodes from the instance down to the node at `path`, both included: ordinary
/// frames and groups, nested instances and the target, in tree order.
pub fn tree_path(doc: &Document, path: &InstancePath) -> Result<Vec<Guid>, ResolveError> {
    let mut overrides = Overrides::new(Arc::new(definitions(doc)));
    descend(doc, &mut overrides, path).map(|(chain, _, _)| chain)
}

/// Enters every instance along `path` (left entered in `overrides`) and returns the tree
/// chain (as [`tree_path`]), the target node and its resolved data.
fn descend<'d>(
    doc: &'d Document,
    overrides: &mut Overrides,
    path: &InstancePath,
) -> Result<(Vec<Guid>, &'d Node, Val), ResolveError> {
    let mut node = doc
        .node(path.instance())
        .ok_or_else(|| ResolveError::new(Kind::UnknownNode))?;
    if node.kind != "INSTANCE" {
        return Err(ResolveError::new(Kind::NotInstance));
    }
    let mut chain = vec![node.id];
    let mut data = node.data.clone();
    for (index, key) in path.keys().iter().enumerate() {
        if node.kind != "INSTANCE" {
            return Err(ResolveError::new(Kind::UnknownPath));
        }
        let symbol = enter_symbol(doc, overrides, &data, &path.keys()[..index])?;
        let trail =
            find_key(doc, symbol.id, *key).ok_or_else(|| ResolveError::new(Kind::UnknownPath))?;
        node = trail
            .last()
            .and_then(|id| doc.node(*id))
            .ok_or_else(|| ResolveError::new(Kind::UnknownPath))?;
        chain.extend(trail);
        data = overrides
            .resolve(doc, node, &path.keys()[..=index])
            .into_owned();
    }
    Ok((chain, node, data))
}

/// The chain of descendants of `root` down to the one with override key (or guid) `key`, not
/// looking inside instances.
fn find_key(doc: &Document, root: Guid, key: Guid) -> Option<Vec<Guid>> {
    let mut stack: Vec<(Guid, usize)> = doc.children(root).iter().map(|id| (*id, 0)).collect();
    let mut trail = Vec::new();
    // Each node is visited at most once in a tree; the budget also ends any malformed loop.
    let mut budget = doc.nodes.len();
    while let Some((id, depth)) = stack.pop() {
        budget = budget.checked_sub(1)?;
        let node = doc.node(id)?;
        trail.truncate(depth);
        trail.push(id);
        if node.override_key() == key || node.id == key {
            return Some(trail);
        }
        if node.kind != "INSTANCE" {
            stack.extend(doc.children(id).iter().map(|id| (*id, depth + 1)));
        }
    }
    None
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
