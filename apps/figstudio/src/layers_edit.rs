//! Editing from the Layers panel: Cmd/Ctrl and Shift selection, renaming in place, the eye and
//! lock buttons, and dragging rows to reorder or reparent (`Reparent` batches that keep each
//! layer where it is on the page).

use egui::{Id, Rect, Sense, Ui, vec2};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Document, Guid};

use crate::icons::{self, Icon};
use crate::tab::Tab;
use crate::theme;

/// A row as drawn this frame, in list order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Row {
    pub id: Guid,
    pub depth: usize,
    pub rect: Rect,
}

/// What the rows of one frame collect for the panel.
#[derive(Default)]
pub(crate) struct Pass {
    pub rows: Vec<Row>,
    /// A Shift-click: select from the primary layer to this one.
    pub range_to: Option<Guid>,
}

/// A click on a row: Cmd/Ctrl adds or removes it, Shift extends to it, else it alone.
pub(crate) fn click(tab: &mut Tab, pass: &mut Pass, id: Guid, m: egui::Modifiers) {
    if m.command || m.ctrl {
        tab.toggle_selected(id);
    } else if m.shift && tab.selection.is_some() {
        pass.range_to = Some(id);
    } else {
        tab.select(Some(id));
    }
}

/// The rows from `from` to `to` as listed (either way round), `from` first.
pub(crate) fn range(rows: &[Row], from: Guid, to: Guid) -> Vec<Guid> {
    let at = |g: Guid| rows.iter().position(|r| r.id == g);
    let (Some(a), Some(b)) = (at(from), at(to)) else {
        return vec![to];
    };
    let (lo, hi) = (a.min(b), a.max(b));
    let mut out: Vec<Guid> = rows[lo..=hi].iter().map(|r| r.id).collect();
    out.retain(|g| *g != from);
    out.insert(0, from);
    out
}

/// After the rows: the Shift range.
pub(crate) fn finish_selection(tab: &mut Tab, pass: &Pass) {
    if let (Some(to), Some(from)) = (pass.range_to, tab.selection) {
        tab.select_all_of(&range(&pass.rows, from, to));
    }
}

// --- Rename -------------------------------------------------------------------------------

#[derive(Clone)]
struct Rename {
    id: Guid,
    text: String,
    fresh: bool,
}

fn rename_id() -> Id {
    Id::new("layers-rename")
}

pub(crate) fn start_rename(ui: &Ui, id: Guid, name: &str) {
    ui.data_mut(|d| {
        d.insert_temp(
            rename_id(),
            Rename {
                id,
                text: name.to_string(),
                fresh: true,
            },
        );
    });
}

pub(crate) fn renaming(ui: &Ui) -> Option<Guid> {
    ui.data(|d| d.get_temp::<Rename>(rename_id())).map(|r| r.id)
}

/// A layer name is being edited: Escape is the field's (it drops the edit).
pub(crate) fn is_renaming(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<Rename>(rename_id()).is_some())
}

/// The name field over `rect` while `id` is renamed: Enter or a click away keeps the name,
/// Escape drops it.
pub(crate) fn rename_field(ui: &mut Ui, tab: &mut Tab, id: Guid, rect: Rect) {
    let Some(mut r) = ui.data(|d| d.get_temp::<Rename>(rename_id())) else {
        return;
    };
    let edit = ui.put(
        rect,
        egui::TextEdit::singleline(&mut r.text)
            .font(egui::FontId::proportional(12.5))
            .margin(vec2(2.0, 0.0)),
    );
    crate::a11y::name_field(&edit, "Rename layer");
    crate::a11y::author(&edit, "layers.rename");
    if r.fresh {
        edit.request_focus();
        r.fresh = false;
    }
    let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let done = edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter));
    if done || cancel {
        ui.data_mut(|d| d.remove::<Rename>(rename_id()));
        let name = r.text.trim();
        let current = tab.doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
        if !cancel && !name.is_empty() && name != current {
            let _ = crate::editor::apply_tab_ops(
                tab,
                &[EditOp::Rename {
                    target: id,
                    name: name.to_string(),
                }],
                Some("Rename layer"),
                None,
            );
        }
    } else {
        ui.data_mut(|d| d.insert_temp(rename_id(), r));
    }
}

// --- Eye and lock -------------------------------------------------------------------------

/// The eye and lock buttons at the row's right end: shown while the row is hovered, and
/// always when the layer is hidden or locked. Returns the width they take.
pub(crate) fn toggles(ui: &mut Ui, tab: &mut Tab, id: Guid, name: &str, rect: Rect) -> f32 {
    let Some(node) = tab.doc.node(id) else {
        return 0.0;
    };
    let hidden = node.data.flag("visible") == Some(false);
    let locked = node.data.flag("locked") == Some(true);
    let hovered = ui.rect_contains_pointer(rect);
    if !(hovered || hidden || locked) {
        return 0.0;
    }
    let p = theme::of(ui);
    let mut width: f32 = 0.0;
    let mut toggle =
        |ui: &mut Ui, slot: f32, on: bool, show: bool, icon: Icon, label: String, aid: &str| {
            if !show {
                return false;
            }
            let r = Rect::from_center_size(
                egui::pos2(rect.right() - 12.0 - slot * 20.0, rect.center().y),
                vec2(18.0, rect.height()),
            );
            let hit = ui.interact(r, ui.id().with((aid, id)), Sense::click());
            hit.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            crate::a11y::author(&hit, format!("layers.{id}.{aid}"));
            let color = if hit.hovered() {
                p.text
            } else if on {
                p.text_dim
            } else {
                p.text_faint
            };
            icons::paint(
                ui.painter(),
                icon,
                Rect::from_center_size(r.center(), vec2(13.0, 13.0)),
                color,
            );
            width = width.max(slot * 20.0 + 24.0);
            hit.clicked()
        };
    let lock_click = toggle(
        ui,
        0.0,
        locked,
        hovered || locked,
        if locked { Icon::Lock } else { Icon::Unlock },
        format!("{} {name}", if locked { "Unlock" } else { "Lock" }),
        "lock",
    );
    let eye_click = toggle(
        ui,
        1.0,
        hidden,
        hovered || hidden,
        if hidden { Icon::EyeOff } else { Icon::Eye },
        format!("{} {name}", if hidden { "Show" } else { "Hide" }),
        "visible",
    );
    if lock_click || eye_click {
        let (visible, lock) = if eye_click {
            (Some(hidden), None)
        } else {
            (None, Some(!locked))
        };
        let _ = crate::editor::apply_tab_ops(
            tab,
            &[EditOp::SetLayerProps {
                target: id,
                visible,
                locked: lock,
                opacity: None,
                blend_mode: None,
                clip_content: None,
                proportions_locked: None,
            }],
            Some(if eye_click {
                "Show/Hide"
            } else {
                "Lock/Unlock"
            }),
            None,
        );
    }
    width
}

// --- Drag to reorder and reparent ---------------------------------------------------------

fn drag_id() -> Id {
    Id::new("layers-drag")
}

/// Where dragged rows land, relative to a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Drop {
    /// Just above the row in the list (in front of it).
    Above(Guid),
    /// Just below the row (behind it).
    Below(Guid),
    /// Inside the row's layer, at the top.
    Into(Guid),
}

fn container(doc: &Document, id: Guid) -> bool {
    doc.node(id).is_some_and(|n| {
        matches!(
            n.kind.as_str(),
            "FRAME" | "GROUP" | "SECTION" | "SYMBOL" | "BOOLEAN_OPERATION"
        )
    }) && !in_instance(doc, id)
}

/// Inside an instance (its layers come from the component and cannot be rearranged).
fn in_instance(doc: &Document, id: Guid) -> bool {
    let mut cur = doc.node(id).and_then(|n| n.parent);
    for _ in 0..4096 {
        let Some(c) = cur else { return false };
        let Some(n) = doc.node(c) else { return false };
        if n.kind == "INSTANCE" {
            return true;
        }
        cur = n.parent;
    }
    false
}

fn is_or_inside(doc: &Document, id: Guid, of: &[Guid]) -> bool {
    let mut cur = Some(id);
    for _ in 0..4096 {
        let Some(c) = cur else { return false };
        if of.contains(&c) {
            return true;
        }
        cur = doc.node(c).and_then(|n| n.parent);
    }
    false
}

/// A row drag starts: the selection if the row is in it, else the row alone (selected).
pub(crate) fn start_drag(ui: &Ui, tab: &mut Tab, id: Guid) {
    if in_instance(&tab.doc, id) {
        return;
    }
    let ids = if tab.selected().contains(&id) {
        let sel = tab.selected();
        sel.iter()
            .copied()
            .filter(|g| {
                !in_instance(&tab.doc, *g)
                    && !tab
                        .doc
                        .node(*g)
                        .and_then(|n| n.parent)
                        .is_some_and(|p| is_or_inside(&tab.doc, p, &sel))
            })
            .collect()
    } else {
        tab.select(Some(id));
        vec![id]
    };
    ui.data_mut(|d| d.insert_temp(drag_id(), ids));
}

/// The drop under page-space `y` in the panel, with the rect to mark.
pub(crate) fn drop_at(
    doc: &Document,
    rows: &[Row],
    dragged: &[Guid],
    y: f32,
) -> Option<(Drop, Rect)> {
    let row = rows.iter().find(|r| r.rect.y_range().contains(y))?;
    let frac = (y - row.rect.top()) / row.rect.height().max(1.0);
    let into_ok = container(doc, row.id) && !dragged.contains(&row.id);
    let drop = if into_ok {
        match frac {
            f if f < 0.25 => Drop::Above(row.id),
            f if f > 0.75 => Drop::Below(row.id),
            _ => Drop::Into(row.id),
        }
    } else if frac < 0.5 {
        Drop::Above(row.id)
    } else {
        Drop::Below(row.id)
    };
    Some((drop, row.rect))
}

/// The `Reparent` ops that put `dragged` at `drop`, keeping their order (front first, as in the
/// list) and their place on the page; empty when the drop is not allowed or changes nothing.
pub(crate) fn drop_ops(
    doc: &Document,
    dragged: &[Guid],
    drop: Drop,
    open: &dyn Fn(Guid) -> bool,
) -> Vec<EditOp> {
    // Below an open layer with children is its top child's place: inside, at the top.
    let drop = match drop {
        Drop::Below(r) if open(r) && !doc.children(r).is_empty() && container(doc, r) => {
            Drop::Into(r)
        }
        d => d,
    };
    let last_other = |p: Guid| {
        doc.children(p)
            .iter()
            .rev()
            .find(|c| !dragged.contains(c))
            .copied()
    };
    let (parent, after) = match drop {
        Drop::Into(r) => (r, last_other(r)),
        Drop::Above(r) => match doc.node(r).and_then(|n| n.parent) {
            Some(p) => (p, Some(r)),
            None => return Vec::new(),
        },
        Drop::Below(r) => {
            let Some(p) = doc.node(r).and_then(|n| n.parent) else {
                return Vec::new();
            };
            let kids = doc.children(p);
            let at = kids.iter().position(|c| *c == r).unwrap_or(0);
            (
                p,
                kids[..at]
                    .iter()
                    .rev()
                    .find(|c| !dragged.contains(c))
                    .copied(),
            )
        }
    };
    if is_or_inside(doc, parent, dragged)
        || in_instance(doc, parent)
        || doc.node(parent).is_some_and(|n| n.kind == "INSTANCE")
        || after.is_some_and(|a| dragged.contains(&a))
    {
        return Vec::new();
    }
    // Back to front: each lands just in front of the one before.
    let mut order: Vec<Guid> = dragged.to_vec();
    order.reverse();
    let mut list: Vec<Guid> = doc.children(parent).to_vec();
    let before = list.clone();
    let mut ops = Vec::new();
    for (i, d) in order.iter().enumerate() {
        list.retain(|g| g != d);
        let pos = (after
            .and_then(|a| list.iter().position(|g| *g == a))
            .map_or(0, |p| p + 1)
            + i)
            .min(list.len());
        list.insert(pos, *d);
        ops.push(EditOp::Reparent {
            target: *d,
            new_parent: parent,
            index: Some(pos),
            keep_position: true,
        });
    }
    let same_parent = dragged
        .iter()
        .all(|d| doc.node(*d).and_then(|n| n.parent) == Some(parent));
    if same_parent && list == before {
        return Vec::new();
    }
    ops
}

/// While rows are dragged: marks the drop and applies it on release.
pub(crate) fn drag_rows(ui: &mut Ui, tab: &mut Tab, rows: &[Row]) {
    let Some(dragged) = ui.data(|d| d.get_temp::<Vec<Guid>>(drag_id())) else {
        return;
    };
    let pointer = ui.input(|i| i.pointer.latest_pos());
    let released = ui.input(|i| !i.pointer.primary_down());
    // In list order (front first), whatever order they were selected in.
    let mut dragged = dragged;
    dragged.sort_by_key(|g| rows.iter().position(|r| r.id == *g).unwrap_or(usize::MAX));
    let doc = tab.doc.clone();
    let target = pointer.and_then(|p| drop_at(&doc, rows, &dragged, p.y));
    if let Some((drop, rect)) = target {
        let depth = rows.iter().find(|r| r.rect == rect).map_or(0, |r| r.depth);
        let left = rect.left() + 6.0 + depth as f32 * 14.0;
        let stroke = egui::Stroke::new(2.0, theme::ACCENT);
        match drop {
            Drop::Above(_) => {
                ui.painter().hline(left..=rect.right(), rect.top(), stroke);
            }
            Drop::Below(_) => {
                ui.painter()
                    .hline(left..=rect.right(), rect.bottom(), stroke);
            }
            Drop::Into(_) => {
                ui.painter()
                    .rect_stroke(rect.shrink(1.0), 6.0, stroke, egui::StrokeKind::Inside);
            }
        }
    }
    if released {
        ui.data_mut(|d| d.remove::<Vec<Guid>>(drag_id()));
        if let Some((drop, _)) = target {
            let expanded = tab.expanded.clone();
            let ops = drop_ops(&doc, &dragged, drop, &|g| expanded.contains(&g));
            if !ops.is_empty() {
                let label = if dragged.len() == 1 {
                    "Move layer"
                } else {
                    "Move layers"
                };
                if crate::editor::apply_tab_ops(tab, &ops, Some(label), None).is_ok() {
                    tab.select_all_of(&dragged);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "layers_edit_tests.rs"]
mod tests;
