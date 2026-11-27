//! Batch execution, paste transaction merging, and ancestor-scoped Auto-Layout derive pass.

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::clipboard::{decode_clipboard_html, paste_kiwi_into};
use crate::edit::autolayout::append_auto_layout_patches;
use crate::edit::geometry::{SHAPE_GEOMETRY_INPUTS, patch_touches_any, sync_derived_caches_in_txn};
use crate::edit::styles_vars::import_image_bytes;
use crate::edit::{Dirty, GuidAlloc, History, NodePatch, Txn};
use crate::{Document, FigError, Guid, Obj};

use super::compile_components::recompute_boolean_node;
use super::compile_tree::compile_single_op;
use super::{EditBatchReport, EditOp};

/// Executes `ops` atomically against `doc` and `history` (or simulates without mutating when
/// `dry_run` is `true`).
pub fn execute_edit_ops(
    doc: &mut Document,
    history: &mut History,
    ops: &[EditOp],
    label: Option<&str>,
    coalesce_key: Option<&str>,
    dry_run: bool,
) -> Result<(EditBatchReport, Dirty), FigError> {
    if ops.is_empty() {
        return Ok((
            EditBatchReport {
                applied: !dry_run,
                revision: doc.revision,
                undo_depth: history.undo_len(),
                redo_depth: history.redo_len(),
                ..Default::default()
            },
            Dirty::default(),
        ));
    }

    let mut staged = clone_document_shell(doc);
    let mut alloc = GuidAlloc::for_document(&staged);
    let mut batch_txn = Txn::new(label.unwrap_or("Apply edits"));
    if let Some(ck) = coalesce_key {
        batch_txn = batch_txn.with_coalesce(ck);
    }

    let mut created_ids = Vec::new();
    let mut touched_for_layout: Vec<Guid> = Vec::new();

    for op in ops {
        let mut step_txn = Txn::new("step");
        compile_single_op(
            &staged,
            &mut step_txn,
            &mut alloc,
            op,
            &mut created_ids,
            &mut touched_for_layout,
        )?;
        if !step_txn.is_empty() {
            apply_step_and_repair_booleans(
                &mut staged,
                &mut batch_txn,
                step_txn,
                &mut touched_for_layout,
            )?;
        }
    }

    // Derive Pass: solve Auto-Layout strictly on the topmost Auto-Layout ancestors of touched
    // nodes, repairing any booleans affected by layout updates and re-solving if needed.
    let al_adjustments = run_autolayout_pass(&mut staged, &mut batch_txn, &touched_for_layout)?;

    if dry_run {
        let mut dirty_pages: Vec<Guid> = batch_txn
            .patches
            .iter()
            .filter_map(|(id, _)| staged.page_of(*id).or_else(|| doc.page_of(*id)))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        dirty_pages.sort();
        let mut touched: Vec<Guid> = batch_txn
            .patches
            .iter()
            .map(|(id, _)| *id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        touched.sort();
        return Ok((
            EditBatchReport {
                applied: false,
                revision: doc.revision,
                created_ids,
                touched_nodes: touched,
                dirty_pages,
                auto_layout_adjustments: al_adjustments,
                undo_depth: history.undo_len(),
                redo_depth: history.redo_len(),
            },
            Dirty::default(),
        ));
    }

    let dirty = history.execute(doc, batch_txn)?;
    let mut touched_nodes: Vec<Guid> = dirty.nodes.iter().copied().collect();
    touched_nodes.sort();
    let mut dirty_pages: Vec<Guid> = dirty.pages.iter().copied().collect();
    dirty_pages.sort();

    Ok((
        EditBatchReport {
            applied: true,
            revision: doc.revision,
            created_ids,
            touched_nodes,
            dirty_pages,
            auto_layout_adjustments: al_adjustments,
            undo_depth: history.undo_len(),
            redo_depth: history.redo_len(),
        },
        dirty,
    ))
}

/// Imports an image into `doc` via `history` and returns `(sha1_hex, Dirty)`.
pub fn execute_import_image(
    doc: &mut Document,
    history: &mut History,
    bytes: Vec<u8>,
) -> Result<(String, Dirty), FigError> {
    let mut txn = Txn::new("Import image");
    let hash = import_image_bytes(&mut txn, bytes)?;
    let dirty = history.execute(doc, txn)?;
    Ok((hash, dirty))
}

/// Pastes Figma HTML clipboard (or base64 kiwi) into `doc` via `history` as a single undoable
/// [`Txn`] (merging any resulting Auto-Layout adjustments into the same history entry).
pub fn execute_paste_html(
    doc: &mut Document,
    history: &mut History,
    html: &str,
    target_parent: Guid,
    offset: (f64, f64),
) -> Result<(Vec<Guid>, Dirty), FigError> {
    let payload = decode_clipboard_html(html)?;
    let (mut batch_txn, roots) = paste_kiwi_into(doc, &payload.kiwi_bytes, target_parent, offset)?;
    for (hash, bytes) in payload.images {
        if !doc.images.contains_key(&hash) && !doc.lazy_images.contains_key(&hash) {
            batch_txn.add_images.push((hash, bytes));
        }
    }
    let mut staged = clone_document_shell(doc);
    staged.apply_txn(&batch_txn)?;
    let mut layout_touched = vec![target_parent];
    let mut paste_dirty = HashSet::new();
    for &root in &roots {
        collect_visible_boolean_ancestors(&staged, root, false, &mut paste_dirty);
    }
    recompute_dirty_booleans(
        &mut staged,
        &mut batch_txn,
        &paste_dirty,
        &mut layout_touched,
    )?;
    run_autolayout_pass(&mut staged, &mut batch_txn, &layout_touched)?;
    let dirty = history.execute(doc, batch_txn)?;
    Ok((roots, dirty))
}

const BOOLEAN_OPERAND_EXTRA_KEYS: &[&str] = &[
    "parentIndex",
    "transform",
    "visible",
    "fillGeometry",
    "booleanOperation",
    "type",
];

fn patch_affects_boolean_operand(set: &Obj, unset: &[String]) -> bool {
    patch_touches_any(set, unset, BOOLEAN_OPERAND_EXTRA_KEYS)
        || patch_touches_any(set, unset, SHAPE_GEOMETRY_INPUTS)
}

/// Collects `BOOLEAN_OPERATION` ancestors reachable from `start` without crossing a hidden node.
fn collect_visible_boolean_ancestors(
    doc: &Document,
    start: Guid,
    include_self: bool,
    out: &mut HashSet<Guid>,
) {
    let mut cur = Some(start);
    let mut seen = HashSet::new();
    let mut first = true;
    while let Some(id) = cur {
        if !seen.insert(id) {
            break;
        }
        let Some(node) = doc.node(id) else { break };
        if (!first || include_self) && node.kind == "BOOLEAN_OPERATION" {
            out.insert(id);
        }
        if node.data.flag("visible") == Some(false) {
            break;
        }
        first = false;
        cur = node.parent;
    }
}

fn collect_step_booleans(
    doc: &Document,
    step_txn: &Txn,
    after: bool,
    out: &mut HashSet<Guid>,
) -> Result<(), FigError> {
    for (id, patch) in &step_txn.patches {
        match patch {
            NodePatch::Delete if !after => collect_visible_boolean_ancestors(doc, *id, false, out),
            NodePatch::Create { .. } if after => {
                collect_visible_boolean_ancestors(doc, *id, false, out);
            }
            NodePatch::Modify { set, unset } => {
                let mut ancestors = HashSet::new();
                if patch_affects_boolean_operand(set, unset) {
                    collect_visible_boolean_ancestors(doc, *id, false, &mut ancestors);
                }
                if !after
                    && !ancestors.is_empty()
                    && patch_touches_any(set, unset, &["size"])
                    && !set.contains_key("fillGeometry")
                    && !unset.iter().any(|u| u == "fillGeometry")
                    && doc
                        .node(*id)
                        .is_some_and(|n| n.kind == "VECTOR" && n.data.get("fillGeometry").is_some())
                {
                    return Err(FigError::malformed(format!(
                        "cannot resize VECTOR operand {id} inside BOOLEAN_OPERATION without vectorData.vectorNetworkBlob and normalizedSize"
                    )));
                }
                out.extend(ancestors);
                if doc.node(*id).is_some_and(|n| n.kind == "BOOLEAN_OPERATION")
                    && !set.contains_key("fillGeometry")
                {
                    if patch_touches_any(set, unset, &["size"]) {
                        return Err(FigError::malformed(format!(
                            "cannot resize BOOLEAN_OPERATION {id}: derived boolean geometry cannot be resized without operand scaling"
                        )));
                    }
                    if (after && set.contains_key("type"))
                        || patch_touches_any(set, unset, &["booleanOperation"])
                        || patch_touches_any(set, unset, SHAPE_GEOMETRY_INPUTS)
                    {
                        out.insert(*id);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn apply_step_and_repair_booleans(
    staged: &mut Document,
    batch_txn: &mut Txn,
    mut step_txn: Txn,
    touched_for_layout: &mut Vec<Guid>,
) -> Result<(), FigError> {
    sync_derived_caches_in_txn(staged, &mut step_txn);
    let mut dirty = HashSet::new();
    collect_step_booleans(staged, &step_txn, false, &mut dirty)?;
    staged.apply_txn(&step_txn)?;

    let mut after = HashSet::new();
    collect_step_booleans(staged, &step_txn, true, &mut after)?;
    let created: HashSet<Guid> = step_txn
        .patches
        .iter()
        .filter_map(|(id, p)| matches!(p, NodePatch::Create { .. }).then_some(*id))
        .collect();
    dirty.extend(after.into_iter().filter(|id| !created.contains(id)));

    merge_step_into_batch(batch_txn, step_txn);
    recompute_dirty_booleans(staged, batch_txn, &dirty, touched_for_layout)
}

fn recompute_dirty_booleans(
    staged: &mut Document,
    batch_txn: &mut Txn,
    dirty_booleans: &HashSet<Guid>,
    touched_for_layout: &mut Vec<Guid>,
) -> Result<(), FigError> {
    if dirty_booleans.is_empty() {
        return Ok(());
    }
    let mut closed = HashSet::new();
    for &id in dirty_booleans {
        collect_visible_boolean_ancestors(staged, id, true, &mut closed);
    }
    let depth_of = |mut cur: Option<Guid>| -> usize {
        let (mut d, mut seen) = (0usize, HashSet::new());
        while let Some(pid) = cur.and_then(|id| staged.node(id)).and_then(|n| n.parent) {
            if !seen.insert(pid) {
                break;
            }
            d += 1;
            cur = Some(pid);
        }
        d
    };
    let mut ordered: Vec<(usize, Guid)> = closed
        .into_iter()
        .filter(|&id| {
            staged
                .node(id)
                .is_some_and(|n| n.kind == "BOOLEAN_OPERATION")
        })
        .map(|id| (depth_of(Some(id)), id))
        .collect();
    ordered.sort_by_key(|&(depth, id)| (Reverse(depth), id));

    for (_, bool_id) in ordered {
        let mut bool_txn = Txn::new("recompute boolean");
        recompute_boolean_node(staged, bool_id, &mut bool_txn)?;
        if !bool_txn.is_empty() {
            staged.apply_txn(&bool_txn)?;
            merge_step_into_batch(batch_txn, bool_txn);
            touched_for_layout.push(bool_id);
        }
    }
    Ok(())
}

fn run_autolayout_pass(
    staged: &mut Document,
    batch_txn: &mut Txn,
    initial_touched: &[Guid],
) -> Result<usize, FigError> {
    let mut al_adjustments = 0usize;
    let mut pending = initial_touched.to_vec();
    for _ in 0..4 {
        if pending.is_empty() {
            break;
        }
        let mut next_pending = Vec::new();
        for root in topmost_autolayout_ancestors(staged, &pending) {
            let mut al_txn = Txn::new("autolayout");
            al_adjustments += append_auto_layout_patches(staged, Some(root), &mut al_txn);
            if !al_txn.is_empty() {
                apply_step_and_repair_booleans(staged, batch_txn, al_txn, &mut next_pending)?;
            }
        }
        pending = next_pending;
    }
    if !pending.is_empty() {
        return Err(FigError::malformed(
            "auto-layout and boolean recomputation did not converge",
        ));
    }
    Ok(al_adjustments)
}

/// Finds the deduplicated topmost Auto-Layout container ancestors covering `touched`.
fn topmost_autolayout_ancestors(doc: &Document, touched: &[Guid]) -> Vec<Guid> {
    let mut candidates: HashSet<Guid> = HashSet::new();
    for &start in touched {
        let mut cur = Some(start);
        let mut seen = HashSet::new();
        let mut highest_al: Option<Guid> = None;
        while let Some(id) = cur {
            if !seen.insert(id) {
                break;
            }
            let Some(node) = doc.node(id) else { break };
            if matches!(
                node.data.text("stackMode"),
                Some("HORIZONTAL" | "VERTICAL" | "GRID")
            ) {
                highest_al = Some(id);
            }
            cur = node.parent;
        }
        if let Some(top) = highest_al {
            candidates.insert(top);
        }
    }
    // Filter out any candidate that still has another candidate as an ancestor.
    let mut roots: Vec<Guid> = candidates
        .iter()
        .copied()
        .filter(|&id| {
            let mut p = doc.node(id).and_then(|n| n.parent);
            let mut seen = HashSet::new();
            while let Some(anc) = p {
                if !seen.insert(anc) {
                    break;
                }
                if candidates.contains(&anc) {
                    return false;
                }
                p = doc.node(anc).and_then(|n| n.parent);
            }
            true
        })
        .collect();
    roots.sort();
    roots
}

pub(super) fn merge_step_into_batch(batch: &mut Txn, step: Txn) {
    batch.patches.extend(step.patches);
    batch.add_blobs.extend(step.add_blobs);
    for (h, b) in step.add_images {
        if !batch.add_images.iter().any(|(eh, _)| eh == &h) {
            batch.add_images.push((h, b));
        }
    }
}

fn clone_document_shell(doc: &Document) -> Document {
    Document {
        instance: doc.instance,
        lineage: doc.lineage,
        image_instance: doc.image_instance,
        revision: doc.revision,
        prelude: doc.prelude.clone(),
        format_version: doc.format_version,
        file_name: doc.file_name.clone(),
        nodes: doc.nodes.clone(),
        index: doc.index.clone(),
        children: doc.children.clone(),
        root: doc.root,
        pages: doc.pages.clone(),
        internal_pages: doc.internal_pages.clone(),
        blobs: doc.blobs.clone(),
        images: Default::default(),
        lazy_images: Default::default(),
        thumbnail: None,
        source: doc.source.clone(),
    }
}

/// Returns concise JSON-schema-like documentation of all supported [`EditOp`] variants for MCP agents.
pub fn describe_edit_ops_markdown() -> &'static str {
    concat!(
        "# Supported `apply_edits` Operations (`op` tag)\n\n",
        "- `create_shape`: `{ kind: FRAME|SECTION|GROUP|ROUNDED_RECTANGLE|ELLIPSE|LINE|REGULAR_POLYGON|STAR, parent, name?, x, y, width, height, corner_radius?, fill_hex?, count?, star_inner_scale? }`\n",
        "- `create_text`: `{ parent, x, y, characters, font_family?, font_style?, font_size?, fill_hex? }`\n",
        "- `create_page`: `{ name }`\n",
        "- `delete_node`: `{ target }`\n",
        "- `duplicate_node`: `{ target, dx?, dy? }`\n",
        "- `rename`: `{ target, name }`\n",
        "- `set_layer_props`: `{ target, visible?, locked?, opacity?, blend_mode?, clip_content? }`\n",
        "- `move`: `{ target, x, y, relative? }`\n",
        "- `resize`: `{ target, width, height }` (propagates constraints & rebuilds geometry)\n",
        "- `rotate`: `{ target, degrees }`\n",
        "- `reparent`: `{ target, new_parent, index? }`\n",
        "- `set_corner_radii`: `{ target, radii: [tl, tr, br, bl], smoothing? }`\n",
        "- `set_constraints`: `{ target, horizontal?: MIN|CENTER|MAX|STRETCH|SCALE, vertical? }`\n",
        "- `set_auto_layout`: `{ target, mode: NONE|HORIZONTAL|VERTICAL|GRID, spacing?, counter_spacing?, padding?: [top,right,bottom,left], primary_sizing?: FIXED|HUG, counter_sizing?: FIXED|HUG, primary_align?: MIN|CENTER|MAX|SPACE_BETWEEN, counter_align?: MIN|CENTER|MAX|BASELINE, wrap? }`\n",
        "- `set_auto_layout_child`: `{ target, primary_fill?, counter_stretch?, positioning?: AUTO|ABSOLUTE, min_size?: [w,h], max_size?: [w,h] }`\n",
        "- `set_solid_fill`: `{ target, hex: \"#RRGGBB\" | null }`\n",
        "- `set_image_fill`: `{ target, sha1_hex, scale_mode?: FILL|FIT|CROP|TILE, opacity? }`\n",
        "- `set_stroke`: `{ target, hex: \"#RRGGBB\" | null, weight?, align?: INSIDE|CENTER|OUTSIDE }`\n",
        "- `set_effects`: `{ target, drop_shadow?: { inner?, rgba:[r,g,b,a], offset:[x,y], radius, spread? }, blur_radius?, background_blur? }`\n",
        "- `set_text`: `{ target, characters }`\n",
        "- `splice_text`: `{ target, utf16_start, utf16_delete_len, insert }`\n",
        "- `style_text_range`: `{ target, utf16_start, utf16_end, style: { font_family?, font_style?, font_size?, fill_hex?, text_decoration?, text_case?, letter_spacing_px?, line_height_px? } }`\n",
        "- `set_typography`: `{ target, font_family?, font_style?, font_size?, align_horizontal?, auto_resize? }`\n",
        "- `set_instance_override`: `{ instance, override_path: [\"1:2\", ...], characters?, visible?, fill_hex?, opacity? }`\n",
        "- `reset_instance_override`: `{ instance, override_path?, field? }`\n",
        "- `set_component_prop`: `{ instance, prop, kind: bool|text|instance_swap, value }`\n",
        "- `switch_variant`: `{ instance, variants: [[\"Size\", \"Large\"]] }`\n",
        "- `swap_instance`: `{ instance, symbol }`\n",
        "- `detach_instance`: `{ instance }`\n",
        "- `create_component`: `{ frame }` / `instantiate_component`: `{ symbol, parent, x, y }`\n",
        "- `bind_style`: `{ target, slot: fill|stroke|text|effect|grid, style: \"1:2\" | null }` / `create_fill_style`: `{ name, hex }`\n",
        "- `bind_paint_variable`: `{ target, field?: fillPaints|strokePaints, paint_index?, variable: \"1:2\" | null }`\n",
        "- `set_variable_mode`: `{ container, collection, mode: \"1:2\" | null }`\n",
        "- `set_variable_color`: `{ variable, mode, hex }`\n",
        "- `set_vector_network`: `{ target, network }` / `boolean_combine`: `{ op: Union|Subtract|Intersect|Exclude, a, b, name? }`\n",
        "- `patch_raw`: `{ target, set: {...}, unset: [...] }`\n"
    )
}
