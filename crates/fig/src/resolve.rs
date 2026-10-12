//! Instance overrides: what the children of an instance look like after `symbolOverrides`,
//! `derivedSymbolData`, component properties and swaps. The renderer and every other reader
//! share this merge, so they never disagree about an instance's content.
//!
//! [`Overrides`] is the low-level scope stack a walker keeps while it descends through nested
//! instances; [`walk_instance`], [`resolve_instance`] and [`resolve_path`] are the high-level
//! entry points built on it.
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use crate::{Document, Guid, Node, Obj, Val};

mod properties;
mod scale;
mod swap;
mod types;
mod walk;

pub use properties::{Definitions, definitions};
pub use scale::{STROKE_LENGTHS, scale_inherited, uniform_scale};
pub use swap::inherit_swapped;
pub use walk::{
    InstancePath, MAX_RESOLVED_NODES, ResolveError, Resolved, Walk, resolve_instance, resolve_path,
    tree_path, walk_instance, walk_path,
};

/// Instances nested deeper than this are not expanded. Guards against cycles and exponential
/// fan-out in hostile files; real files nest a handful of levels. The renderer counts each
/// cut in `Stats::depth_skipped`, the walker in [`Walk::skipped_instances`].
pub const MAX_INSTANCE_DEPTH: usize = 24;

/// A node's style reference and the paint list that the style replaces.
pub const STYLED_PAINTS: [(&str, &str); 2] = [
    ("styleIdForFill", "fillPaints"),
    ("styleIdForStrokeFill", "strokePaints"),
];

/// One override entry, keyed by its full override path.
#[derive(Debug)]
pub struct Patch {
    /// The override fields (`guidPath` is ignored by the merge).
    pub data: Val,
    /// How many instances enclose the instance that owns this patch (0 = outermost).
    pub owner_depth: usize,
    /// From `derivedSymbolData` (Figma's computed layout) rather than `symbolOverrides`.
    pub derived: bool,
}

#[derive(Debug)]
struct Frame {
    symbol: Guid,
    /// The instance's `uniformScaleFactor`.
    scale: f64,
    patches_before: usize,
    keys: Vec<Vec<Guid>>,
}

/// Override scopes of the instances a walk is currently inside, outermost first.
#[derive(Debug)]
pub struct Overrides {
    patches: Vec<Patch>,
    index: HashMap<Vec<Guid>, Vec<usize>>,
    properties: properties::Properties,
    frames: Vec<Frame>,
}

/// The component an instance shows: a swap (`overriddenSymbolID`) or its own `symbolID`.
pub fn symbol_id(data: &Val) -> Option<Guid> {
    data.get("overriddenSymbolID")
        .and_then(Val::as_guid)
        .or_else(|| {
            data.path(&["symbolData", "symbolID"])
                .and_then(Val::as_guid)
        })
}

impl Overrides {
    /// An empty scope stack; `definitions` comes from [`definitions`] for the same document.
    pub fn new(definitions: Arc<Definitions>) -> Self {
        Self {
            patches: Vec::new(),
            index: HashMap::new(),
            properties: properties::Properties::new(definitions),
            frames: Vec::new(),
        }
    }

    /// Number of instances entered.
    pub fn depth(&self) -> usize {
        self.frames.len()
    }

    /// False when `symbol` is already being expanded (a cycle) or the nesting cap is reached.
    pub fn can_enter(&self, symbol: Guid) -> bool {
        self.frames.len() < MAX_INSTANCE_DEPTH && self.frames.iter().all(|f| f.symbol != symbol)
    }

    /// Enters an instance (resolved `data`) of `symbol` whose own override path is `base`
    /// (empty for an instance that is not itself inside an instance). Undo with [`Self::leave`].
    pub fn enter(&mut self, symbol: &Node, data: &Val, base: &[Guid]) {
        let owner_depth = self.frames.len();
        let patches_before = self.patches.len();
        let mut keys = Vec::new();
        let lists = [
            (data.path(&["symbolData", "symbolOverrides"]), false),
            (data.get("derivedSymbolData"), true),
        ];
        for (list, derived) in lists {
            for item in list.map(Val::items).unwrap_or_default() {
                let guids = item.path(&["guidPath", "guids"]).map(Val::items);
                let guids = guids.unwrap_or_default().iter().filter_map(Val::as_guid);
                let mut full = base.to_vec();
                let len = full.len();
                full.extend(guids);
                if full.len() == len {
                    continue;
                }
                self.index
                    .entry(full.clone())
                    .or_default()
                    .push(self.patches.len());
                keys.push(full);
                self.patches.push(Patch {
                    data: item.clone(),
                    owner_depth,
                    derived,
                });
            }
        }
        self.properties.push(symbol, data);
        self.frames.push(Frame {
            symbol: symbol.id,
            scale: uniform_scale(data),
            patches_before,
            keys,
        });
    }

    /// Leaves the innermost entered instance.
    pub fn leave(&mut self) {
        let Some(frame) = self.frames.pop() else {
            return;
        };
        self.properties.pop();
        self.patches.truncate(frame.patches_before);
        for key in frame.keys {
            let remove = self.index.get_mut(&key).is_some_and(|indexes| {
                indexes.pop();
                indexes.is_empty()
            });
            if remove {
                self.index.remove(&key);
            }
        }
    }

    /// Patches for the node at override path `path`, in merge order: outer instances last so
    /// they win, and derived data after the overrides of the same instance.
    pub fn matches(&self, path: &[Guid]) -> Vec<&Patch> {
        let mut matches: Vec<&Patch> = (self.index.get(path).into_iter().flatten())
            .filter_map(|index| self.patches.get(*index))
            .collect();
        matches.sort_by_key(|p| (std::cmp::Reverse(p.owner_depth), p.derived));
        matches
    }

    /// Product of the uniform scales of every entered instance.
    pub fn scale(&self) -> f64 {
        self.frames.iter().map(|f| f.scale).product()
    }

    /// The factor that scales `field` of a node with patches `matches`: a value comes from the
    /// outermost level that states it, already in that level's scaled space, so only the
    /// uniformly scaled instances outside that level still scale it.
    pub fn outer_scale(&self, matches: &[&Patch], field: &str) -> f64 {
        let level = matches
            .iter()
            .filter(|p| p.data.get(field).is_some())
            .map(|p| p.owner_depth)
            .min()
            .unwrap_or(self.frames.len());
        self.frames[..level.min(self.frames.len())]
            .iter()
            .map(|f| f.scale)
            .product()
    }

    /// Fields set on `data`'s node by component properties of the innermost instance.
    pub fn property_patch(&self, data: &Val) -> Val {
        self.properties.patch(data)
    }

    /// The node's data after overrides, uniform scale ([`scale_inherited`]) and swaps,
    /// borrowed when nothing applies. This is the renderer's merge without its constraint and
    /// glyph-freshness corrections.
    pub fn resolve<'n>(&self, doc: &Document, node: &'n Node, path: &[Guid]) -> Cow<'n, Val> {
        let matches = self.matches(path);
        let property_patch = self.property_patch(&node.data);
        if matches.is_empty() && property_patch == Val::Obj(Obj::new()) && self.scale() == 1.0 {
            return Cow::Borrowed(&node.data);
        }
        let mut data = merge(&node.data, &property_patch, &matches, |_, _| {});
        scale_inherited(&mut data, |field| self.outer_scale(&matches, field));
        inherit_swapped(doc, &node.data, &mut data);
        Cow::Owned(data)
    }
}

/// `base` with the property patch, then each patch in order. `observe` sees the data before
/// each patch is merged. An explicit paint override replaces the paints of a style it does not
/// restate, so the style reference is dropped. Instance swaps are a separate step
/// ([`inherit_swapped`]) because the renderer scales inherited lengths before it.
pub fn merge(
    base: &Val,
    property_patch: &Val,
    matches: &[&Patch],
    mut observe: impl FnMut(&Val, &Patch),
) -> Val {
    let mut data = base.merged(property_patch);
    for p in matches {
        observe(&data, p);
        data = data.merged(&p.data);
        for (style, key) in STYLED_PAINTS {
            if p.data.get(key).is_some()
                && p.data.get(style).is_none()
                && let Val::Obj(o) = &mut data
            {
                o.remove(style);
            }
        }
    }
    data
}
