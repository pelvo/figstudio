//! Walks an instance's resolved children and looks up one child by its Figma-style id path.
use std::backtrace::Backtrace;
use std::fmt;
use std::sync::Arc;

use super::{Overrides, definitions, symbol_id};
use crate::{Document, Guid, Node, Val};

/// A node inside an instance, as Figma names it: `I<instance>;<key>[;<key>...]`. The first
/// guid is the instance in the document; each further one is the override key (`overrideKey`,
/// else the guid) of a node in the component the previous instance shows. Ordinary frames and
/// groups between them are not part of the path, only nested instances.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InstancePath(Vec<Guid>);

impl InstancePath {
    /// `None` unless there is an instance and at least one key.
    pub fn new(guids: Vec<Guid>) -> Option<Self> {
        (guids.len() >= 2).then_some(Self(guids))
    }

    /// The instance in the document.
    pub fn instance(&self) -> Guid {
        self.0[0]
    }

    /// The override path inside the instance (every guid after the first).
    pub fn keys(&self) -> &[Guid] {
        &self.0[1..]
    }
}

impl fmt::Display for InstancePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("I")?;
        for (index, guid) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(";")?;
            }
            write!(f, "{guid}")?;
        }
        Ok(())
    }
}

/// One node inside an instance with every override applied.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// Figma-style id of this node.
    pub path: InstancePath,
    /// The component node this one is drawn from.
    pub node: Guid,
    /// The component node's data with overrides, properties and swaps applied.
    pub data: Val,
    /// Tree depth below the walked instance (its direct children are at level 1).
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
#[derive(Debug)]
enum Step {
    Visit(Guid, Vec<Guid>, usize),
    Children(Guid, Vec<Guid>, usize),
    Leave,
}

/// Default node cap of [`resolve_instance`]; bounds work on hostile, exponentially nested files.
pub const MAX_RESOLVED_NODES: usize = 100_000;

/// Every node inside `instance`, at most [`MAX_RESOLVED_NODES`].
pub fn resolve_instance(doc: &Document, instance: Guid) -> Result<Walk, ResolveError> {
    walk_instance(doc, instance, None, MAX_RESOLVED_NODES)
}

/// Nodes inside `instance` down to `max_level` (unbounded when `None`), at most `max_nodes`.
pub fn walk_instance(
    doc: &Document,
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
    let mut overrides = Overrides::new(Arc::new(definitions(doc)));
    let mut walk = Walk::default();
    let mut stack = Vec::new();
    if max_level.is_none_or(|limit| limit >= 1) {
        enter(doc, &mut overrides, &node.data, Vec::new(), 1, &mut stack)?;
    }
    while let Some(step) = stack.pop() {
        match step {
            Step::Leave => overrides.leave(),
            Step::Children(parent, base, level) => {
                for child in doc.children(parent).iter().rev() {
                    stack.push(Step::Visit(*child, base.clone(), level));
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
                    stack.push(Step::Children(id, base, level + 1));
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
    Ok(walk)
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
    stack.push(Step::Children(symbol.id, base, level));
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
    let instance = doc
        .node(path.instance())
        .ok_or_else(|| ResolveError::new(Kind::UnknownNode))?;
    if instance.kind != "INSTANCE" {
        return Err(ResolveError::new(Kind::NotInstance));
    }
    let mut overrides = Overrides::new(Arc::new(definitions(doc)));
    let mut data = instance.data.clone();
    let mut node = instance;
    for (index, key) in path.keys().iter().enumerate() {
        if node.kind != "INSTANCE" {
            return Err(ResolveError::new(Kind::UnknownPath));
        }
        let symbol = enter_symbol(doc, &mut overrides, &data, &path.keys()[..index])?;
        node =
            find_key(doc, symbol.id, *key).ok_or_else(|| ResolveError::new(Kind::UnknownPath))?;
        data = overrides
            .resolve(doc, node, &path.keys()[..=index])
            .into_owned();
    }
    Ok(Resolved {
        path: path.clone(),
        node: node.id,
        data,
        level: path.keys().len(),
    })
}

/// The descendant of `root` with override key (or guid) `key`, not looking inside instances.
fn find_key(doc: &Document, root: Guid, key: Guid) -> Option<&Node> {
    let mut stack: Vec<Guid> = doc.children(root).to_vec();
    // Each node is visited at most once in a tree; the budget also ends any malformed loop.
    let mut budget = doc.nodes.len();
    while let Some(id) = stack.pop() {
        budget = budget.checked_sub(1)?;
        let node = doc.node(id)?;
        if node.override_key() == key || node.id == key {
            return Some(node);
        }
        if node.kind != "INSTANCE" {
            stack.extend(doc.children(id));
        }
    }
    None
}

/// Why an instance or instance path could not be resolved.
#[derive(Debug)]
pub struct ResolveError {
    kind: Kind,
    backtrace: Backtrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    UnknownNode,
    NotInstance,
    MissingComponent,
    TooDeep,
    UnknownPath,
}

impl ResolveError {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            backtrace: Backtrace::capture(),
        }
    }

    /// The instance, or a key of the path, names no node at that place.
    pub fn is_not_found(&self) -> bool {
        matches!(self.kind, Kind::UnknownNode | Kind::UnknownPath)
    }

    /// The captured backtrace (empty unless `RUST_BACKTRACE` is set).
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.kind {
            Kind::UnknownNode => "the instance does not exist in this file",
            Kind::NotInstance => "the first id of an instance path must be an instance",
            Kind::MissingComponent => "the instance's component is missing from this file",
            Kind::TooDeep => "instances nest too deeply or form a cycle",
            Kind::UnknownPath => "the instance path names no node inside the instance",
        })
    }
}

impl std::error::Error for ResolveError {}
