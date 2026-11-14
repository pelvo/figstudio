//! Low-level atomic transactions, mechanical inverse generation, dirty tracking, and
//! Undo/Redo history for [`Document`](crate::Document).

use std::collections::{HashMap, HashSet};

use crate::edit::schema_check::{validate_node_data, validate_node_field};
use crate::{Document, FigError, Guid, Node, Obj, Val};

/// Low-level mutation applied to a single [`Node`] inside a [`Txn`].
#[derive(Clone, Debug)]
pub enum NodePatch {
    /// Sets and/or removes top-level `NodeChange` fields on an existing node.
    Modify { set: Obj, unset: Vec<String> },
    /// Creates a new node (at `at_index` when restoring a deleted node during undo, or
    /// appended to `Document::nodes` when `None`).
    Create { at_index: Option<usize>, data: Val },
    /// Deletes an existing node from `Document::nodes`.
    Delete,
}

impl NodePatch {
    /// Convenience constructor for creating a new node at the end of `nodeChanges`.
    pub fn create(data: Val) -> Self {
        Self::Create {
            at_index: None,
            data,
        }
    }

    /// Convenience constructor for setting fields on an existing node.
    pub fn set_fields(set: Obj) -> Self {
        Self::Modify {
            set,
            unset: Vec::new(),
        }
    }
}

/// An atomic document transaction with mechanical undo support.
#[derive(Clone, Debug, Default)]
pub struct Txn {
    pub label: String,
    /// Optional key used by [`History`] to coalesce consecutive interactive drag ticks
    /// (for example `"drag:move:1:42"`) into a single undo entry.
    pub coalesce_key: Option<String>,
    pub patches: Vec<(Guid, NodePatch)>,
    /// Blobs appended to `Document::blobs` when this transaction is applied.
    pub add_blobs: Vec<Vec<u8>>,
    /// When set (on an inverse transaction), truncates `Document::blobs` back to this length.
    pub truncate_blobs_to: Option<usize>,
    /// Embedded images (`(sha1_hex, bytes)`) inserted into `Document::images`.
    pub add_images: Vec<(String, Vec<u8>)>,
    /// Image hashes removed when applying an inverse transaction.
    pub remove_images: Vec<String>,
}

impl Txn {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn with_coalesce(mut self, key: impl Into<String>) -> Self {
        self.coalesce_key = Some(key.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
            && self.add_blobs.is_empty()
            && self.truncate_blobs_to.is_none()
            && self.add_images.is_empty()
            && self.remove_images.is_empty()
    }

    pub fn modify(&mut self, id: Guid, set: Obj, unset: Vec<String>) {
        if set.is_empty() && unset.is_empty() {
            return;
        }
        self.patches.push((id, NodePatch::Modify { set, unset }));
    }

    pub fn set_field(&mut self, id: Guid, key: &str, val: Val) {
        let mut set = Obj::with_capacity(1);
        set.insert(key, val);
        self.modify(id, set, Vec::new());
    }

    pub fn unset_field(&mut self, id: Guid, key: impl Into<String>) {
        self.modify(id, Obj::new(), vec![key.into()]);
    }

    pub fn create_node(&mut self, id: Guid, mut data: Val) {
        data.set("guid", id.to_val());
        self.patches.push((id, NodePatch::create(data)));
    }

    pub fn delete_node(&mut self, id: Guid) {
        self.patches.push((id, NodePatch::Delete));
    }

    /// Merges `later` onto `self` (used when coalescing drag preview ticks into one undo step).
    /// Note: `self` and `later` here are *inverse* transactions where `later` was captured
    /// after `self`; to undo both in reverse order, `later` runs first, then `self`.
    pub fn prepend_earlier_inverse(mut later_inv: Txn, earlier_inv: Txn) -> Txn {
        later_inv.patches.extend(earlier_inv.patches);
        if earlier_inv.truncate_blobs_to.is_some() {
            later_inv.truncate_blobs_to = earlier_inv.truncate_blobs_to;
        }
        later_inv.add_blobs.extend(earlier_inv.add_blobs);
        for (h, b) in earlier_inv.add_images {
            later_inv.remove_images.retain(|x| x != &h);
            if !later_inv.add_images.iter().any(|(eh, _)| eh == &h) {
                later_inv.add_images.push((h, b));
            }
        }
        for h in earlier_inv.remove_images {
            if !later_inv.add_images.iter().any(|(eh, _)| eh == &h)
                && !later_inv.remove_images.contains(&h)
            {
                later_inv.remove_images.push(h);
            }
        }
        later_inv
    }
}

/// Scope of changes produced by [`Document::apply_txn`], used to invalidate only affected
/// pages/tiles and avoid discarding decoded image caches when images did not change.
#[derive(Clone, Debug, Default)]
pub struct Dirty {
    pub nodes: HashSet<Guid>,
    pub pages: HashSet<Guid>,
    pub images_changed: bool,
}

/// Bounded undo/redo stack for an open [`Document`].
#[derive(Clone, Debug)]
struct UndoEntry {
    label: String,
    coalesce: Option<String>,
    inv: Txn,
    before_state: u64,
    after_state: u64,
}

#[derive(Clone, Debug)]
struct RedoEntry {
    label: String,
    txn: Txn,
    before_state: u64,
    after_state: u64,
}

#[derive(Clone, Debug)]
pub struct History {
    undo_stack: Vec<UndoEntry>,
    redo_stack: Vec<RedoEntry>,
    max_entries: usize,
    state_id: u64,
    next_state_id: u64,
}

impl Default for History {
    fn default() -> Self {
        Self::new(200)
    }
}

impl History {
    pub fn new(max_entries: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_entries: max_entries.max(1),
            state_id: 0,
            next_state_id: 1,
        }
    }

    /// Exact logical document state identifier across commits, Undos, and Redos (unlike
    /// [`Document::revision`], which increments monotonically to invalidate render caches).
    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }

    pub fn undo_labels(&self) -> Vec<&str> {
        self.undo_stack.iter().map(|e| e.label.as_str()).collect()
    }

    pub fn redo_labels(&self) -> Vec<&str> {
        self.redo_stack.iter().map(|e| e.label.as_str()).collect()
    }

    /// Applies `txn` to `doc` and records its mechanical inverse on the undo stack,
    /// coalescing consecutive transactions that share a non-`None` `coalesce_key`.
    pub fn execute(&mut self, doc: &mut Document, txn: Txn) -> Result<Dirty, FigError> {
        if txn.is_empty() {
            return Ok(Dirty::default());
        }
        let label = txn.label.clone();
        let coalesce = txn.coalesce_key.clone();
        let (inv, dirty) = doc.apply_txn(&txn)?;
        self.redo_stack.clear();
        let prev_state = self.state_id;
        let new_state = self.next_state_id;
        self.next_state_id += 1;
        self.state_id = new_state;

        if let Some(ref key) = coalesce
            && let Some(last) = self.undo_stack.last_mut()
            && last.coalesce.as_ref() == Some(key)
        {
            let combined = Txn::prepend_earlier_inverse(inv, std::mem::take(&mut last.inv));
            last.inv = combined;
            last.after_state = new_state;
            return Ok(dirty);
        }

        if self.undo_stack.len() >= self.max_entries {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(UndoEntry {
            label,
            coalesce,
            inv,
            before_state: prev_state,
            after_state: new_state,
        });
        Ok(dirty)
    }

    /// Breaks coalescing so the next drag gesture starts a fresh undo entry.
    pub fn seal_coalesce(&mut self) {
        if let Some(last) = self.undo_stack.last_mut() {
            last.coalesce = None;
        }
    }

    /// Undoes the most recent committed entry.
    pub fn undo(&mut self, doc: &mut Document) -> Result<Option<(String, Dirty)>, FigError> {
        let Some(entry) = self.undo_stack.pop() else {
            return Ok(None);
        };
        let (redo_txn, dirty) = match doc.apply_txn(&entry.inv) {
            Ok(res) => res,
            Err(e) => {
                self.undo_stack.push(UndoEntry {
                    coalesce: None,
                    ..entry
                });
                return Err(e);
            }
        };
        self.state_id = entry.before_state;
        let label = entry.label;
        self.redo_stack.push(RedoEntry {
            label: label.clone(),
            txn: redo_txn,
            before_state: entry.before_state,
            after_state: entry.after_state,
        });
        Ok(Some((label, dirty)))
    }

    /// Redoes the most recently undone entry.
    pub fn redo(&mut self, doc: &mut Document) -> Result<Option<(String, Dirty)>, FigError> {
        let Some(entry) = self.redo_stack.pop() else {
            return Ok(None);
        };
        let (inv, dirty) = match doc.apply_txn(&entry.txn) {
            Ok(res) => res,
            Err(e) => {
                self.redo_stack.push(entry);
                return Err(e);
            }
        };
        self.state_id = entry.after_state;
        let label = entry.label;
        self.undo_stack.push(UndoEntry {
            label: label.clone(),
            coalesce: None,
            inv,
            before_state: entry.before_state,
            after_state: entry.after_state,
        });
        Ok(Some((label, dirty)))
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

impl Document {
    /// Validates `txn` against this document's Kiwi schema and parent-cycle rules, applies it
    /// atomically, and returns `(inverse_txn, dirty_scope)`.
    ///
    /// If validation or cycle checking fails, `self` is left completely untouched.
    pub fn apply_txn(&mut self, txn: &Txn) -> Result<(Txn, Dirty), FigError> {
        if txn.is_empty() {
            return Ok((Txn::new(format!("Undo {}", txn.label)), Dirty::default()));
        }

        let schema = self.decoded_schema()?;

        // 1. Validate every patch up front against the file's Kiwi schema and existence rules.
        let mut simulated_exists: HashSet<Guid> = self.index.keys().copied().collect();
        for (id, patch) in &txn.patches {
            let at = format!("node {id}");
            match patch {
                NodePatch::Modify { set, unset } => {
                    if !simulated_exists.contains(id) {
                        return Err(FigError::malformed(format!(
                            "{at}: cannot modify missing node"
                        )));
                    }
                    if unset.iter().any(|k| k == "guid") {
                        return Err(FigError::malformed(format!("{at}: cannot unset guid")));
                    }
                    if let Some(g) = set.get("guid").and_then(Val::as_guid)
                        && g != *id
                    {
                        return Err(FigError::malformed(format!(
                            "{at}: cannot change node guid to {g}"
                        )));
                    }
                    for (k, v) in set.iter() {
                        validate_node_field(&schema, k, v, &at)?;
                    }
                }
                NodePatch::Create { data, .. } => {
                    if simulated_exists.contains(id) {
                        return Err(FigError::malformed(format!(
                            "{at}: node with guid already exists"
                        )));
                    }
                    let data_guid = data.get("guid").and_then(Val::as_guid);
                    if data_guid != Some(*id) {
                        return Err(FigError::malformed(format!(
                            "{at}: created data guid {data_guid:?} does not match {id}"
                        )));
                    }
                    validate_node_data(&schema, data, &at)?;
                    simulated_exists.insert(*id);
                }
                NodePatch::Delete => {
                    if !simulated_exists.remove(id) {
                        return Err(FigError::malformed(format!(
                            "{at}: cannot delete missing node"
                        )));
                    }
                    if self.root == Some(*id) {
                        return Err(FigError::malformed(format!(
                            "{at}: cannot delete DOCUMENT root"
                        )));
                    }
                }
            }
        }

        // 2. Stage nodes so parent-cycle checks run before committing any mutation to `self`.
        let mut staged_nodes = self.nodes.clone();
        let mut staged_index = self.index.clone();
        let mut dirty = Dirty::default();
        let mut inv_patches: Vec<(Guid, NodePatch)> = Vec::with_capacity(txn.patches.len());

        // Record pre-edit pages of touched nodes so moved/deleted nodes mark their old page dirty.
        for (id, _) in &txn.patches {
            dirty.nodes.insert(*id);
            if let Some(p) = self.page_of(*id) {
                dirty.pages.insert(p);
            }
        }

        for (id, patch) in &txn.patches {
            match patch {
                NodePatch::Modify { set, unset } => {
                    let idx = staged_index[id];
                    let node = &mut staged_nodes[idx];
                    let Val::Obj(ref mut map) = node.data else {
                        return Err(FigError::malformed(format!(
                            "node {id}: data is not an object"
                        )));
                    };
                    let mut inv_set = Obj::new();
                    let mut inv_unset = Vec::new();
                    let mut touched_keys = HashSet::new();

                    for key in unset {
                        if touched_keys.insert(key.clone())
                            && let Some(old_raw) = map.remove(key)
                        {
                            inv_set.insert(key, old_raw);
                        }
                    }
                    for (key, new_val) in set.iter() {
                        let first_touch = touched_keys.insert(key.to_string());
                        match map.insert(key, new_val.clone()) {
                            Some(old_raw) if first_touch => {
                                inv_set.insert(key, old_raw);
                            }
                            None if first_touch => {
                                inv_unset.push(key.to_string());
                            }
                            _ => {}
                        }
                    }
                    let rebuilt = Node::from_data(node.data.clone()).ok_or_else(|| {
                        FigError::malformed(format!("node {id}: invalid node after modify"))
                    })?;
                    *node = rebuilt;
                    inv_patches.push((
                        *id,
                        NodePatch::Modify {
                            set: inv_set,
                            unset: inv_unset,
                        },
                    ));
                }
                NodePatch::Create { at_index, data } => {
                    let node = Node::from_data(data.clone()).ok_or_else(|| {
                        FigError::malformed(format!("node {id}: invalid created node"))
                    })?;
                    match at_index {
                        Some(pos) if *pos <= staged_nodes.len() => {
                            staged_nodes.insert(*pos, node);
                            rebuild_index(&staged_nodes, &mut staged_index);
                        }
                        _ => {
                            staged_index.insert(*id, staged_nodes.len());
                            staged_nodes.push(node);
                        }
                    }
                    inv_patches.push((*id, NodePatch::Delete));
                }
                NodePatch::Delete => {
                    let idx = staged_index[id];
                    let removed = staged_nodes.remove(idx);
                    rebuild_index(&staged_nodes, &mut staged_index);
                    inv_patches.push((
                        *id,
                        NodePatch::Create {
                            at_index: Some(idx),
                            data: removed.data,
                        },
                    ));
                }
            }
        }

        // Reject dangling children pointing to deleted parents? Wait: in Figma if a subtree is
        // deleted, all descendants should be deleted in the same Txn, or orphaned nodes must not
        // form cycles. Verify cycle-freedom using the same check as `Document::set_nodes`.
        crate::doc::check_parent_links(&staged_nodes, &staged_index)?;

        // Inverse patches must run in reverse order of application!
        inv_patches.reverse();

        // 3. Apply blobs and images.
        let blobs_before = self.blobs.len();
        let mut inv_truncate_blobs = None;
        let mut inv_add_blobs = Vec::new();
        if let Some(trunc) = txn.truncate_blobs_to
            && trunc < self.blobs.len()
        {
            inv_add_blobs = self.blobs[trunc..].to_vec();
            self.blobs.truncate(trunc);
        }
        if !txn.add_blobs.is_empty() {
            if inv_truncate_blobs.is_none() {
                inv_truncate_blobs = Some(blobs_before.min(self.blobs.len()));
            }
            self.blobs.extend(txn.add_blobs.iter().cloned());
        }

        let mut inv_add_images = Vec::new();
        let mut inv_remove_images = Vec::new();
        for hash in &txn.remove_images {
            if let Some(prev_bytes) = self.images.remove(hash) {
                dirty.images_changed = true;
                inv_add_images.push((hash.clone(), prev_bytes));
            }
        }
        for (hash, bytes) in &txn.add_images {
            let key = hash.to_ascii_lowercase();
            match self.images.insert(key.clone(), bytes.clone()) {
                Some(prev_bytes) => {
                    if prev_bytes != *bytes {
                        dirty.images_changed = true;
                    }
                    inv_add_images.push((key, prev_bytes));
                }
                None => {
                    dirty.images_changed = true;
                    inv_remove_images.push(key);
                }
            }
        }

        // 4. Commit staged nodes and update revision / instance numbers.
        self.nodes = staged_nodes;
        self.index = staged_index;
        self.bump_after_edit(dirty.images_changed);

        // Also record post-edit pages of touched nodes (e.g. created or reparented across pages).
        for (id, _) in &txn.patches {
            if let Some(p) = self.page_of(*id) {
                dirty.pages.insert(p);
            }
        }

        let inv = Txn {
            label: format!("Undo {}", txn.label),
            coalesce_key: None,
            patches: inv_patches,
            add_blobs: inv_add_blobs,
            truncate_blobs_to: inv_truncate_blobs,
            add_images: inv_add_images,
            remove_images: inv_remove_images,
        };
        Ok((inv, dirty))
    }
}

fn rebuild_index(nodes: &[Node], index: &mut HashMap<Guid, usize>) {
    index.clear();
    index.reserve(nodes.len());
    for (i, n) in nodes.iter().enumerate() {
        index.insert(n.id, i);
    }
}
