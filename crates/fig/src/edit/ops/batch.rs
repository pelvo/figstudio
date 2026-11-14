//! Batch execution, paste transaction merging, and ancestor-scoped Auto-Layout derive pass.

use std::collections::HashSet;

use crate::clipboard::{decode_clipboard_html, paste_kiwi_into};
use crate::edit::autolayout::{append_auto_layout_patches, compute_auto_layout};
use crate::edit::styles_vars::import_image_bytes;
use crate::edit::{Dirty, GuidAlloc, History, Txn};
use crate::{Document, FigError, Guid};

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
            staged.apply_txn(&step_txn)?;
            merge_step_into_batch(&mut batch_txn, step_txn);
        }
    }

    // Derive Pass: solve Auto-Layout strictly on the topmost Auto-Layout ancestors of touched
    // nodes — never across unrelated frames or pages in the document!
    let mut al_adjustments = 0usize;
    for root in topmost_autolayout_ancestors(&staged, &touched_for_layout) {
        let mut al_txn = Txn::new("autolayout");
        al_adjustments += compute_auto_layout(&staged, Some(root)).len();
        append_auto_layout_patches(&staged, Some(root), &mut al_txn);
        if !al_txn.is_empty() {
            staged.apply_txn(&al_txn)?;
            merge_step_into_batch(&mut batch_txn, al_txn);
        }
    }

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
    let mut staged = clone_document_shell(doc);
    staged.apply_txn(&batch_txn)?;
    for al_root in topmost_autolayout_ancestors(&staged, &[target_parent]) {
        let mut al_txn = Txn::new("Paste Auto-Layout");
        append_auto_layout_patches(&staged, Some(al_root), &mut al_txn);
        if !al_txn.is_empty() {
            staged.apply_txn(&al_txn)?;
            merge_step_into_batch(&mut batch_txn, al_txn);
        }
    }
    let dirty = history.execute(doc, batch_txn)?;
    Ok((roots, dirty))
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
