//! Interactive GUI editor: canvas 8-handle resize & move gizmos, shape/text creation tools,
//! bidirectional editable Properties Inspector, two-way Figma HTML clipboard integration, and
//! Live MCP mutation handler.

use std::sync::Arc;

use egui::{
    Color32, CornerRadius, CursorIcon, DragValue, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2,
};
use figstudio_fig::clipboard::encode_clipboard_html;
use figstudio_fig::edit::{EditOp, execute_edit_ops, execute_paste_html};
use figstudio_fig::{Guid, Val};

use crate::canvas::Tool;
use crate::tab::Tab;
use crate::theme;

/// Which of the 8 resize handles around a selected layer is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::NW | Self::SE => CursorIcon::ResizeNwSe,
            Self::NE | Self::SW => CursorIcon::ResizeNeSw,
            Self::N | Self::S => CursorIcon::ResizeVertical,
            Self::E | Self::W => CursorIcon::ResizeHorizontal,
        }
    }

    pub fn screen_pos(self, r: Rect) -> Pos2 {
        match self {
            Self::NW => r.left_top(),
            Self::N => pos2(r.center().x, r.top()),
            Self::NE => r.right_top(),
            Self::E => pos2(r.right(), r.center().y),
            Self::SE => r.right_bottom(),
            Self::S => pos2(r.center().x, r.bottom()),
            Self::SW => r.left_bottom(),
            Self::W => pos2(r.left(), r.center().y),
        }
    }
}

/// Active interactive gesture on the canvas.
#[derive(Clone, Debug)]
pub enum CanvasDrag {
    Move {
        target: Guid,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        moved: bool,
    },
    Resize {
        target: Guid,
        handle: Handle,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        orig_w: f64,
        orig_h: f64,
    },
    Create {
        tool: Tool,
        start_page: Pos2,
        cur_page: Pos2,
    },
}

/// Applies `ops` onto `tab.doc` via `tab.history`, invalidating derived caches and marking dirty.
pub fn apply_tab_ops(
    tab: &mut Tab,
    ops: &[EditOp],
    label: Option<&str>,
    coalesce_key: Option<&str>,
) -> Result<Vec<Guid>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let (report, _) = execute_edit_ops(doc_mut, &mut tab.history, ops, label, coalesce_key, false)
        .map_err(|e| e.to_string())?;
    if report.applied {
        tab.dirty_unsaved = true;
        tab.motion_preview = None;
    }
    Ok(report.created_ids)
}

/// Undoes the last edit on `tab`.
pub fn undo_tab(tab: &mut Tab) -> Result<Option<String>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let res = tab.history.undo(doc_mut).map_err(|e| e.to_string())?;
    if let Some((label, _)) = res {
        tab.dirty_unsaved = tab.history.can_undo();
        tab.motion_preview = None;
        if let Some(sel) = tab.selection
            && tab.doc.node(sel).is_none()
        {
            tab.selection = None;
            tab.inner = None;
        }
        Ok(Some(label))
    } else {
        Ok(None)
    }
}

/// Redoes the last undone edit on `tab`.
pub fn redo_tab(tab: &mut Tab) -> Result<Option<String>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let res = tab.history.redo(doc_mut).map_err(|e| e.to_string())?;
    if let Some((label, _)) = res {
        tab.dirty_unsaved = true;
        tab.motion_preview = None;
        Ok(Some(label))
    } else {
        Ok(None)
    }
}

/// Copies the current selection as Figma `text/html` clipboard markup (plus plain-text fallback).
pub fn copy_selection_for_figma(ctx: &egui::Context, tab: &Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to copy for Figma".to_string())?;
    let html = encode_clipboard_html(&tab.doc, &[sel], 1, tab.doc_key.as_deref())
        .map_err(|e| e.to_string())?;
    let name = tab
        .doc
        .node(sel)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    write_system_html_clipboard(ctx, &html, &name)?;
    Ok(format!("Copied “{name}” for Figma"))
}

/// Reads Figma `text/html` from the system clipboard and pastes it into `tab`.
pub fn paste_from_system_clipboard(tab: &mut Tab) -> Result<String, String> {
    let html = read_system_html_clipboard()?;
    let target_parent = tab
        .selection
        .filter(|&id| {
            tab.doc.node(id).is_some_and(|n| {
                matches!(n.kind.as_str(), "FRAME" | "SECTION" | "GROUP" | "CANVAS")
            })
        })
        .unwrap_or(tab.page);
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let (roots, _) = execute_paste_html(
        doc_mut,
        &mut tab.history,
        &html,
        target_parent,
        (24.0, 24.0),
    )
    .map_err(|e| e.to_string())?;
    tab.dirty_unsaved = true;
    if let Some(&first) = roots.first() {
        tab.select(Some(first));
    }
    Ok(format!("Pasted {} layer(s)", roots.len()))
}

#[cfg(not(target_arch = "wasm32"))]
fn write_system_html_clipboard(
    _ctx: &egui::Context,
    html: &str,
    plain: &str,
) -> Result<(), String> {
    use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
    let cb = ClipboardContext::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    cb.set(vec![
        ClipboardContent::Html(html.to_owned()),
        ClipboardContent::Text(plain.to_owned()),
    ])
    .map_err(|e| format!("Could not write Figma HTML clipboard: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn write_system_html_clipboard(
    ctx: &egui::Context,
    html: &str,
    _plain: &str,
) -> Result<(), String> {
    ctx.copy_text(html.to_owned());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_system_html_clipboard() -> Result<String, String> {
    use clipboard_rs::{Clipboard, ClipboardContext};
    let cb = ClipboardContext::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    if let Ok(h) = cb.get_html()
        && h.contains("(figma)")
    {
        return Ok(h);
    }
    if let Ok(t) = cb.get_text()
        && t.contains("(figma)")
    {
        return Ok(t);
    }
    Err("Clipboard does not contain Figma layer data".into())
}

#[cfg(target_arch = "wasm32")]
fn read_system_html_clipboard() -> Result<String, String> {
    Err("Use MCP paste_from_figma or browser paste event in web mode".into())
}

/// Handles canvas creation tools, 8-handle resize dragging, and direct selection dragging,
/// returning `Some(cursor)` when an editor gizmo owns pointer interaction this frame.
pub fn canvas_gizmos_and_input(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    tool: &mut Tool,
    panning: bool,
) -> Option<CursorIcon> {
    if panning || tab.preview.is_some() {
        return None;
    }

    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
    let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    // 1. Creation tools (`Frame`, `Section`, `Rectangle`, `Ellipse`, `Line`, `Text`).
    if tool.is_creation() {
        if primary_pressed
            && let Some(pos) = pointer_pos
            && rect.contains(pos)
        {
            let p = tab.view.to_page(rect.min, pos);
            tab.edit_drag = Some(CanvasDrag::Create {
                tool: *tool,
                start_page: p,
                cur_page: p,
            });
        }
        if let Some(CanvasDrag::Create {
            tool: create_tool,
            start_page,
            ref mut cur_page,
        }) = tab.edit_drag
        {
            if let Some(pos) = pointer_pos {
                *cur_page = tab.view.to_page(rect.min, pos);
            }
            let min_p = pos2(start_page.x.min(cur_page.x), start_page.y.min(cur_page.y));
            let max_p = pos2(start_page.x.max(cur_page.x), start_page.y.max(cur_page.y));
            let s0 = tab.view.to_screen(rect.min, min_p);
            let s1 = tab.view.to_screen(rect.min, max_p);
            painter.rect_stroke(
                Rect::from_min_max(s0, s1),
                0.0,
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
            if primary_released {
                let end_p = *cur_page;
                tab.edit_drag = None;
                let w =
                    f64::from((end_p.x - start_page.x).abs()).max(if create_tool == Tool::Text {
                        120.0
                    } else {
                        80.0
                    });
                let h = f64::from((end_p.y - start_page.y).abs())
                    .max(if create_tool == Tool::Line { 0.0 } else { 60.0 });
                let x = f64::from(start_page.x.min(end_p.x));
                let y = f64::from(start_page.y.min(end_p.y));
                let op = match create_tool {
                    Tool::Text => EditOp::CreateText {
                        id: None,
                        parent: tab.page,
                        x,
                        y,
                        characters: "Text".into(),
                        font_family: "Inter".into(),
                        font_style: "Regular".into(),
                        font_size: 20.0,
                        fill_hex: "#111827".into(),
                    },
                    other => EditOp::CreateShape {
                        id: None,
                        kind: other.shape_kind().unwrap_or("FRAME").into(),
                        parent: tab.page,
                        name: None,
                        x,
                        y,
                        width: w,
                        height: h,
                        corner_radius: 0.0,
                        fill_hex: None,
                        count: None,
                        star_inner_scale: None,
                    },
                };
                if let Ok(ids) = apply_tab_ops(tab, &[op], Some("Create layer"), None)
                    && let Some(&new_id) = ids.first()
                {
                    tab.select(Some(new_id));
                }
                *tool = Tool::Select;
            }
        }
        return Some(CursorIcon::Crosshair);
    }

    if *tool != Tool::Select || tab.text_select.is_some() {
        return None;
    }

    // 2. Continue an in-progress Resize or Move drag without walking hit_chain!
    if let Some(drag) = tab.edit_drag.clone() {
        if primary_down && let Some(pos) = pointer_pos {
            let cur_page = tab.view.to_page(rect.min, pos);
            let dx = f64::from(
                cur_page.x
                    - match &drag {
                        CanvasDrag::Move {
                            start_pointer_page, ..
                        } => start_pointer_page.x,
                        CanvasDrag::Resize {
                            start_pointer_page, ..
                        } => start_pointer_page.x,
                        CanvasDrag::Create { start_page, .. } => start_page.x,
                    },
            );
            let dy = f64::from(
                cur_page.y
                    - match &drag {
                        CanvasDrag::Move {
                            start_pointer_page, ..
                        } => start_pointer_page.y,
                        CanvasDrag::Resize {
                            start_pointer_page, ..
                        } => start_pointer_page.y,
                        CanvasDrag::Create { start_page, .. } => start_page.y,
                    },
            );

            match drag {
                CanvasDrag::Move {
                    target,
                    start_pointer_page,
                    orig_x,
                    orig_y,
                    moved,
                } => {
                    if moved || dx.hypot(dy) * f64::from(tab.view.zoom) >= 3.0 {
                        let key = format!("gui:move:{target}");
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::Move {
                                target,
                                x: (orig_x + dx).round(),
                                y: (orig_y + dy).round(),
                                relative: false,
                            }],
                            Some("Move layer"),
                            Some(&key),
                        );
                        tab.edit_drag = Some(CanvasDrag::Move {
                            target,
                            start_pointer_page,
                            orig_x,
                            orig_y,
                            moved: true,
                        });
                        return Some(CursorIcon::Grabbing);
                    }
                }
                CanvasDrag::Resize {
                    target,
                    handle,
                    orig_x,
                    orig_y,
                    orig_w,
                    orig_h,
                    ..
                } => {
                    let (mut nx, mut ny, mut nw, mut nh) = (orig_x, orig_y, orig_w, orig_h);
                    match handle {
                        Handle::E | Handle::NE | Handle::SE => nw = (orig_w + dx).max(1.0),
                        Handle::W | Handle::NW | Handle::SW => {
                            nw = (orig_w - dx).max(1.0);
                            nx = orig_x + (orig_w - nw);
                        }
                        _ => {}
                    }
                    match handle {
                        Handle::S | Handle::SE | Handle::SW => nh = (orig_h + dy).max(1.0),
                        Handle::N | Handle::NW | Handle::NE => {
                            nh = (orig_h - dy).max(1.0);
                            ny = orig_y + (orig_h - nh);
                        }
                        _ => {}
                    }
                    let key = format!("gui:resize:{target}");
                    let _ = apply_tab_ops(
                        tab,
                        &[
                            EditOp::Move {
                                target,
                                x: nx.round(),
                                y: ny.round(),
                                relative: false,
                            },
                            EditOp::Resize {
                                target,
                                width: nw.round(),
                                height: nh.round(),
                            },
                        ],
                        Some("Resize layer"),
                        Some(&key),
                    );
                    paint_selection_handles(painter, rect, tab, target);
                    return Some(handle.cursor());
                }
                CanvasDrag::Create { .. } => {}
            }
        }
        if primary_released || !primary_down {
            tab.history.seal_coalesce();
            tab.edit_drag = None;
        }
    }

    // 3. Paint 8 resize handles around the selected node and detect handle hover / press.
    let sel = tab.selection?;
    let node = tab.doc.node(sel)?;
    if matches!(node.kind.as_str(), "DOCUMENT" | "CANVAS") || node.data.flag("locked") == Some(true)
    {
        return None;
    }

    let bounds = paint_selection_handles(painter, rect, tab, sel)?;
    let pos = pointer_pos?;
    if !rect.contains(pos) {
        return None;
    }

    for h in Handle::ALL {
        let hp = h.screen_pos(bounds);
        let hit = Rect::from_center_size(hp, vec2(10.0, 10.0));
        if hit.contains(pos) {
            if primary_pressed {
                let (ox, oy, ow, oh) = local_xywh(&node.data);
                tab.edit_drag = Some(CanvasDrag::Resize {
                    target: sel,
                    handle: h,
                    start_pointer_page: tab.view.to_page(rect.min, pos),
                    orig_x: ox,
                    orig_y: oy,
                    orig_w: ow,
                    orig_h: oh,
                });
            }
            return Some(h.cursor());
        }
    }

    if primary_pressed && bounds.contains(pos) {
        let (ox, oy, _, _) = local_xywh(&node.data);
        tab.edit_drag = Some(CanvasDrag::Move {
            target: sel,
            start_pointer_page: tab.view.to_page(rect.min, pos),
            orig_x: ox,
            orig_y: oy,
            moved: false,
        });
    }
    None
}

fn paint_selection_handles(
    painter: &egui::Painter,
    canvas_rect: Rect,
    tab: &Tab,
    node_id: Guid,
) -> Option<Rect> {
    let b = figstudio_render::node_bounds(&tab.doc, node_id)?;
    let min = tab
        .view
        .to_screen(canvas_rect.min, pos2(b.x0 as f32, b.y0 as f32));
    let max = tab
        .view
        .to_screen(canvas_rect.min, pos2(b.x1 as f32, b.y1 as f32));
    let screen_rect = Rect::from_min_max(min, max);
    for h in Handle::ALL {
        let c = h.screen_pos(screen_rect);
        let hr = Rect::from_center_size(c, vec2(7.0, 7.0));
        painter.rect_filled(hr, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(
            hr,
            CornerRadius::same(1),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
    Some(screen_rect)
}

fn local_xywh(data: &Val) -> (f64, f64, f64, f64) {
    let x = data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let y = data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let (w, h) = figstudio_render::geom::size(data);
    (x, y, w, h)
}

/// Renders Undo / Redo buttons when the tab has edit history.
pub fn properties_history_bar(ui: &mut Ui, tab: &mut Tab) {
    let can_u = tab.history.can_undo();
    let can_r = tab.history.can_redo();
    if !can_u && !can_r && !tab.dirty_unsaved {
        return;
    }
    let p = theme::of(ui);
    ui.horizontal(|ui| {
        let u_btn = ui.add_enabled(can_u, egui::Button::new("↶ Undo"));
        crate::a11y::author(&u_btn, "editor.undo");
        if u_btn.clicked() {
            let _ = undo_tab(tab);
        }
        let r_btn = ui.add_enabled(can_r, egui::Button::new("↷ Redo"));
        crate::a11y::author(&r_btn, "editor.redo");
        if r_btn.clicked() {
            let _ = redo_tab(tab);
        }
        if tab.dirty_unsaved {
            ui.colored_label(p.text_dim, format!("• rev {}", tab.doc.revision));
        }
    });
}

/// Renders the interactive Edit controls section inside the right Properties panel.
pub fn properties_edit_controls(ui: &mut Ui, tab: &mut Tab) {
    properties_history_bar(ui, tab);

    // Inner-instance text override quick editor when an inner node inside an INSTANCE is selected!
    if let (Some(inst_id), Some(inner)) = (tab.selection, tab.inner.clone())
        && let Ok(resolved) = figstudio_fig::resolve::resolve_path(&tab.doc, &inner)
        && let Some(chars) = resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str)
    {
        let mut text_buf = chars.to_string();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Instance Text Override")
                .strong()
                .size(11.5),
        );
        let resp = ui.text_edit_multiline(&mut text_buf);
        crate::a11y::name_field(&resp, "Instance text override");
        crate::a11y::author(&resp, "editor.instance-text");
        if resp.changed() {
            let ov_path = inner.keys().to_vec();
            let _ = apply_tab_ops(
                tab,
                &[EditOp::SetInstanceOverride {
                    instance: inst_id,
                    override_path: ov_path,
                    characters: Some(text_buf),
                    visible: None,
                    fill_hex: None,
                    opacity: None,
                }],
                Some("Edit instance text"),
                Some("gui:inst:text"),
            );
        }
        return;
    }

    let Some(sel) = tab.selection else {
        return;
    };
    let Some(node_snapshot) = tab.doc.node(sel).cloned() else {
        return;
    };
    if matches!(node_snapshot.kind.as_str(), "DOCUMENT" | "CANVAS") {
        return;
    }

    let hdr = egui::CollapsingHeader::new(egui::RichText::new("Edit Layer").strong().size(12.0))
        .default_open(false)
        .show(ui, |ui| {
            // 1. Name, Visibility, Duplicate, Delete
            let mut name_buf = node_snapshot.name.clone();
            ui.horizontal(|ui| {
                ui.label("Name");
                let resp = ui.text_edit_singleline(&mut name_buf);
                crate::a11y::name_field(&resp, "Layer name");
                crate::a11y::author(&resp, "editor.layer-name");
                if resp.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::Rename {
                            target: sel,
                            name: name_buf,
                        }],
                        Some("Rename layer"),
                        Some("gui:rename"),
                    );
                }
            });

            let mut vis = node_snapshot.data.flag("visible") != Some(false);
            let mut locked = node_snapshot.data.flag("locked") == Some(true);
            ui.horizontal(|ui| {
                let r_vis = ui.checkbox(&mut vis, "Visible");
                crate::a11y::author(&r_vis, "editor.visible");
                if r_vis.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetLayerProps {
                            target: sel,
                            visible: Some(vis),
                            locked: None,
                            opacity: None,
                            blend_mode: None,
                            clip_content: None,
                        }],
                        Some("Toggle visibility"),
                        None,
                    );
                }
                let r_lock = ui.checkbox(&mut locked, "Lock");
                crate::a11y::author(&r_lock, "editor.lock");
                if r_lock.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetLayerProps {
                            target: sel,
                            visible: None,
                            locked: Some(locked),
                            opacity: None,
                            blend_mode: None,
                            clip_content: None,
                        }],
                        Some("Toggle lock"),
                        None,
                    );
                }
                let r_dup = ui.small_button("Duplicate");
                crate::a11y::author(&r_dup, "editor.duplicate");
                if r_dup.clicked()
                    && let Ok(ids) = apply_tab_ops(
                        tab,
                        &[EditOp::DuplicateNode {
                            target: sel,
                            dx: 24.0,
                            dy: 24.0,
                        }],
                        Some("Duplicate"),
                        None,
                    )
                    && let Some(&dup) = ids.first()
                {
                    tab.select(Some(dup));
                    return;
                }
                let r_del = ui.small_button("Delete");
                crate::a11y::author(&r_del, "editor.delete");
                if r_del.clicked() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::DeleteNode { target: sel }],
                        Some("Delete"),
                        None,
                    );
                    tab.select(None);
                }
            });
            if tab.selection != Some(sel) {
                return;
            }

            // 2. X, Y, W, H, Rotation, Corner Radius
            let (mut x, mut y, mut w, mut h) = local_xywh(&node_snapshot.data);
            let mut pos_changed = false;
            let mut size_changed = false;
            ui.horizontal(|ui| {
                let rx = ui.add(DragValue::new(&mut x).prefix("X ").speed(1.0));
                crate::a11y::name_field(&rx, "Position X");
                crate::a11y::author(&rx, "editor.pos-x");
                pos_changed |= rx.changed();
                let ry = ui.add(DragValue::new(&mut y).prefix("Y ").speed(1.0));
                crate::a11y::name_field(&ry, "Position Y");
                crate::a11y::author(&ry, "editor.pos-y");
                pos_changed |= ry.changed();
            });
            ui.horizontal(|ui| {
                let rw = ui.add(
                    DragValue::new(&mut w)
                        .prefix("W ")
                        .speed(1.0)
                        .range(0.0..=100_000.0),
                );
                crate::a11y::name_field(&rw, "Width");
                crate::a11y::author(&rw, "editor.width");
                size_changed |= rw.changed();
                let rh = ui.add(
                    DragValue::new(&mut h)
                        .prefix("H ")
                        .speed(1.0)
                        .range(0.0..=100_000.0),
                );
                crate::a11y::name_field(&rh, "Height");
                crate::a11y::author(&rh, "editor.height");
                size_changed |= rh.changed();
            });
            if pos_changed {
                let _ = apply_tab_ops(
                    tab,
                    &[EditOp::Move {
                        target: sel,
                        x,
                        y,
                        relative: false,
                    }],
                    Some("Set position"),
                    Some("gui:inspector:pos"),
                );
            }
            if size_changed {
                let _ = apply_tab_ops(
                    tab,
                    &[EditOp::Resize {
                        target: sel,
                        width: w,
                        height: h,
                    }],
                    Some("Set size"),
                    Some("gui:inspector:size"),
                );
            }

            let mut radius = node_snapshot.data.num("cornerRadius").unwrap_or(0.0);
            let mut opacity = node_snapshot.data.num("opacity").unwrap_or(1.0) * 100.0;
            ui.horizontal(|ui| {
                let rr = ui.add(
                    DragValue::new(&mut radius)
                        .prefix("R ")
                        .speed(0.5)
                        .range(0.0..=2000.0),
                );
                crate::a11y::name_field(&rr, "Corner radius");
                crate::a11y::author(&rr, "editor.corner-radius");
                if rr.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetCornerRadii {
                            target: sel,
                            radii: [radius; 4],
                            smoothing: None,
                        }],
                        Some("Set corner radius"),
                        Some("gui:inspector:radius"),
                    );
                }
                let ro = ui.add(
                    DragValue::new(&mut opacity)
                        .prefix("Opacity ")
                        .suffix("%")
                        .speed(1.0)
                        .range(0.0..=100.0),
                );
                crate::a11y::name_field(&ro, "Layer opacity");
                crate::a11y::author(&ro, "editor.opacity");
                if ro.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetLayerProps {
                            target: sel,
                            visible: None,
                            locked: None,
                            opacity: Some(f64::from(opacity) / 100.0),
                            blend_mode: None,
                            clip_content: None,
                        }],
                        Some("Set opacity"),
                        Some("gui:inspector:opacity"),
                    );
                }
            });

            // 3. Solid Fill Hex Input
            let cur_rgba = first_solid_srgba(&node_snapshot.data).unwrap_or([217, 217, 217, 255]);
            let mut hex_buf = format!("#{:02X}{:02X}{:02X}", cur_rgba[0], cur_rgba[1], cur_rgba[2]);
            ui.horizontal(|ui| {
                ui.label("Fill");
                let rf = ui.add(egui::TextEdit::singleline(&mut hex_buf).desired_width(78.0));
                crate::a11y::name_field(&rf, "Fill colour hex");
                crate::a11y::author(&rf, "editor.fill-hex");
                if rf.changed() && hex_buf.trim().len() >= 7 {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetSolidFill {
                            target: sel,
                            hex: Some(hex_buf),
                        }],
                        Some("Set fill colour"),
                        Some("gui:inspector:fill"),
                    );
                }
                let r_cf = ui.small_button("Clear Fill");
                crate::a11y::author(&r_cf, "editor.clear-fill");
                if r_cf.clicked() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetSolidFill {
                            target: sel,
                            hex: None,
                        }],
                        Some("Remove fill"),
                        None,
                    );
                }
            });

            // 4. Auto-Layout Controls on FRAME / SYMBOL
            if matches!(node_snapshot.kind.as_str(), "FRAME" | "SYMBOL") {
                let cur_mode = node_snapshot
                    .data
                    .text("stackMode")
                    .unwrap_or("NONE")
                    .to_string();
                let mut gap = f64::from(node_snapshot.data.num("stackSpacing").unwrap_or(8.0));
                let mut ph = f64::from(
                    node_snapshot
                        .data
                        .num("stackHorizontalPadding")
                        .unwrap_or(12.0),
                );
                let mut pv = f64::from(
                    node_snapshot
                        .data
                        .num("stackVerticalPadding")
                        .unwrap_or(12.0),
                );
                ui.horizontal(|ui| {
                    ui.label("Auto-Layout");
                    for (lbl, mode, aid) in [
                        ("Off", "NONE", "editor.al-off"),
                        ("Horiz", "HORIZONTAL", "editor.al-horiz"),
                        ("Vert", "VERTICAL", "editor.al-vert"),
                    ] {
                        let rb = ui.selectable_label(cur_mode == mode, lbl);
                        crate::a11y::author(&rb, aid);
                        if rb.clicked() {
                            let _ = apply_tab_ops(
                                tab,
                                &[EditOp::SetAutoLayout {
                                    target: sel,
                                    mode: mode.into(),
                                    spacing: Some(gap),
                                    counter_spacing: None,
                                    padding: Some([pv, ph, pv, ph]),
                                    primary_sizing: Some("HUG".into()),
                                    counter_sizing: Some("HUG".into()),
                                    primary_align: None,
                                    counter_align: None,
                                    wrap: None,
                                }],
                                Some("Set Auto-Layout"),
                                None,
                            );
                        }
                    }
                });
                if cur_mode != "NONE" {
                    let mut al_changed = false;
                    ui.horizontal(|ui| {
                        let rg = ui.add(DragValue::new(&mut gap).prefix("Gap ").speed(1.0));
                        crate::a11y::name_field(&rg, "Auto-layout gap");
                        crate::a11y::author(&rg, "editor.al-gap");
                        al_changed |= rg.changed();
                        let rpx = ui.add(DragValue::new(&mut ph).prefix("PadX ").speed(1.0));
                        crate::a11y::name_field(&rpx, "Auto-layout horizontal padding");
                        crate::a11y::author(&rpx, "editor.al-padx");
                        al_changed |= rpx.changed();
                        let rpy = ui.add(DragValue::new(&mut pv).prefix("PadY ").speed(1.0));
                        crate::a11y::name_field(&rpy, "Auto-layout vertical padding");
                        crate::a11y::author(&rpy, "editor.al-pady");
                        al_changed |= rpy.changed();
                    });
                    if al_changed {
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::SetAutoLayout {
                                target: sel,
                                mode: cur_mode,
                                spacing: Some(gap),
                                counter_spacing: None,
                                padding: Some([pv, ph, pv, ph]),
                                primary_sizing: None,
                                counter_sizing: None,
                                primary_align: None,
                                counter_align: None,
                                wrap: None,
                            }],
                            Some("Adjust Auto-Layout"),
                            Some("gui:inspector:al"),
                        );
                    }
                }
            }

            // 5. Text Content & Font Size Editor on TEXT layers
            if node_snapshot.kind == "TEXT" {
                let mut chars = node_snapshot
                    .data
                    .path(&["textData", "characters"])
                    .and_then(Val::as_str)
                    .unwrap_or("")
                    .to_string();
                let mut font_sz = f64::from(node_snapshot.data.num("fontSize").unwrap_or(16.0));
                ui.label("Text");
                let rt = ui.text_edit_multiline(&mut chars);
                crate::a11y::name_field(&rt, "Text content");
                crate::a11y::author(&rt, "editor.text-content");
                if rt.changed() {
                    let _ = apply_tab_ops(
                        tab,
                        &[EditOp::SetText {
                            target: sel,
                            characters: chars,
                        }],
                        Some("Edit text"),
                        Some("gui:inspector:text"),
                    );
                }
                ui.horizontal(|ui| {
                    let rfs = ui.add(
                        DragValue::new(&mut font_sz)
                            .prefix("Size ")
                            .speed(0.5)
                            .range(1.0..=512.0),
                    );
                    crate::a11y::name_field(&rfs, "Font size");
                    crate::a11y::author(&rfs, "editor.font-size");
                    if rfs.changed() {
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::SetTypography {
                                target: sel,
                                font_family: None,
                                font_style: None,
                                font_size: Some(font_sz),
                                align_horizontal: None,
                                auto_resize: None,
                            }],
                            Some("Set font size"),
                            Some("gui:inspector:fontsize"),
                        );
                    }
                    for (sty, aid) in [
                        ("Regular", "editor.weight-regular"),
                        ("Bold", "editor.weight-bold"),
                    ] {
                        let rb = ui.small_button(sty);
                        crate::a11y::author(&rb, aid);
                        if rb.clicked() {
                            let _ = apply_tab_ops(
                                tab,
                                &[EditOp::SetTypography {
                                    target: sel,
                                    font_family: None,
                                    font_style: Some(sty.into()),
                                    font_size: None,
                                    align_horizontal: None,
                                    auto_resize: None,
                                }],
                                Some("Set font weight"),
                                None,
                            );
                        }
                    }
                });
            }

            // 6. Component / Instance actions
            ui.horizontal(|ui| {
                if node_snapshot.kind == "FRAME" {
                    let rb = ui.small_button("❖ Create Component");
                    crate::a11y::author(&rb, "editor.create-component");
                    if rb.clicked() {
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::CreateComponent { frame: sel }],
                            Some("Create Component"),
                            None,
                        );
                    }
                }
                if node_snapshot.kind == "SYMBOL" {
                    let rb = ui.small_button("◇ Instantiate");
                    crate::a11y::author(&rb, "editor.instantiate-component");
                    if rb.clicked() {
                        let (x, y, w, _) = local_xywh(&node_snapshot.data);
                        if let Ok(ids) = apply_tab_ops(
                            tab,
                            &[EditOp::InstantiateComponent {
                                symbol: sel,
                                parent: tab.page,
                                x: x + w + 32.0,
                                y,
                            }],
                            Some("Instantiate Component"),
                            None,
                        ) && let Some(&inst) = ids.first()
                        {
                            tab.select(Some(inst));
                        }
                    }
                }
                if node_snapshot.kind == "INSTANCE" {
                    let r_rst = ui.small_button("Reset Overrides");
                    crate::a11y::author(&r_rst, "editor.reset-overrides");
                    if r_rst.clicked() {
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::ResetInstanceOverride {
                                instance: sel,
                                override_path: None,
                                field: None,
                            }],
                            Some("Reset Overrides"),
                            None,
                        );
                    }
                    let r_det = ui.small_button("Detach Instance");
                    crate::a11y::author(&r_det, "editor.detach-instance");
                    if r_det.clicked()
                        && let Ok(ids) = apply_tab_ops(
                            tab,
                            &[EditOp::DetachInstance { instance: sel }],
                            Some("Detach Instance"),
                            None,
                        )
                        && let Some(&frame) = ids.first()
                    {
                        tab.select(Some(frame));
                    }
                }
            });
        });
    crate::a11y::author(&hdr.header_response, "editor.toggle-section");
}

fn first_solid_srgba(data: &Val) -> Option<[u8; 4]> {
    let p = data.get("fillPaints")?.items().first()?;
    if p.text("type") != Some("SOLID") {
        return None;
    }
    let c = p.get("color")?;
    let op = p.num("opacity").unwrap_or(1.0);
    Some([
        (c.num("r")? * 255.0).round() as u8,
        (c.num("g")? * 255.0).round() as u8,
        (c.num("b")? * 255.0).round() as u8,
        (c.num("a").unwrap_or(1.0) * op * 255.0).round() as u8,
    ])
}

/// Duplicates the currently selected layer on `tab` with a `(24, 24)` offset.
pub fn duplicate_selection(tab: &mut Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to duplicate".to_string())?;
    let ids = apply_tab_ops(
        tab,
        &[EditOp::DuplicateNode {
            target: sel,
            dx: 24.0,
            dy: 24.0,
        }],
        Some("Duplicate layer"),
        None,
    )?;
    if let Some(&first) = ids.first() {
        tab.select(Some(first));
    }
    Ok("Duplicated layer".into())
}

/// Deletes the currently selected layer on `tab`.
pub fn delete_selection(tab: &mut Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to delete".to_string())?;
    if sel == tab.page {
        return Err("Cannot delete the current page root".into());
    }
    let name = tab
        .doc
        .node(sel)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    apply_tab_ops(
        tab,
        &[EditOp::DeleteNode { target: sel }],
        Some("Delete layer"),
        None,
    )?;
    tab.select(None);
    Ok(format!("Deleted “{name}”"))
}

/// Executes a browser-relay MCP editing tool (`apply_edits`, `undo`, `redo`, `get_history`,
/// `import_image`, `paste_from_figma`) on `tab`.
#[cfg(any(test, target_arch = "wasm32"))]
pub fn handle_wasm_mutation(
    tabs: &mut [Tab],
    tab_id: u64,
    tool: &str,
    args: &serde_json::Value,
) -> Result<figstudio_mcp_core::output::Output, String> {
    use base64::Engine as _;
    use figstudio_mcp_core::output::Output;
    use serde_json::json;

    let tab = tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .ok_or_else(|| "tab closed".to_string())?;

    match tool {
        "apply_edits" => {
            let raw_ops = args
                .get("ops")
                .cloned()
                .ok_or_else(|| "ops array is required".to_string())?;
            let ops: Vec<EditOp> =
                serde_json::from_value(raw_ops).map_err(|e| format!("invalid ops: {e}"))?;
            let label = args.get("label").and_then(serde_json::Value::as_str);
            let dry_run = args
                .get("dryRun")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (report, _) =
                execute_edit_ops(doc_mut, &mut tab.history, &ops, label, None, dry_run)
                    .map_err(|e| e.to_string())?;
            if report.applied {
                tab.dirty_unsaved = true;
                tab.motion_preview = None;
            }
            Ok(Output::structured(
                serde_json::to_value(report).map_err(|e| e.to_string())?,
            ))
        }
        "undo" => {
            let undone = undo_tab(tab)?;
            Ok(Output::structured(json!({
                "undone": undone,
                "revision": tab.doc.revision,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            })))
        }
        "redo" => {
            let redone = redo_tab(tab)?;
            Ok(Output::structured(json!({
                "redone": redone,
                "revision": tab.doc.revision,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            })))
        }
        "get_history" => Ok(Output::structured(json!({
            "revision": tab.doc.revision,
            "undo": tab.history.undo_labels(),
            "redo": tab.history.redo_labels(),
        }))),
        "import_image" => {
            let b64 = args
                .get("dataBase64")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "dataBase64 is required".to_string())?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| format!("invalid base64 image data: {e}"))?;
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (hash, _) =
                figstudio_fig::edit::execute_import_image(doc_mut, &mut tab.history, bytes)
                    .map_err(|e| e.to_string())?;
            tab.dirty_unsaved = true;
            Ok(Output::structured(
                json!({ "sha1Hex": hash, "revision": doc_mut.revision }),
            ))
        }
        "paste_from_figma" => {
            let html = args
                .get("html")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "html is required".to_string())?;
            let parent = match args.get("parentId").and_then(serde_json::Value::as_str) {
                Some(raw) => raw.parse::<Guid>().map_err(|e| e.to_string())?,
                None => tab.page,
            };
            let dx = args
                .get("dx")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(24.0);
            let dy = args
                .get("dy")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(24.0);
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (roots, _) = execute_paste_html(doc_mut, &mut tab.history, html, parent, (dx, dy))
                .map_err(|e| e.to_string())?;
            tab.dirty_unsaved = true;
            if let Some(&first) = roots.first() {
                tab.select(Some(first));
            }
            Ok(Output::structured(json!({
                "pastedRootIds": roots.into_iter().map(|g| g.to_string()).collect::<Vec<_>>(),
                "revision": tab.doc.revision,
            })))
        }
        other => Err(format!("Unsupported mutation tool: {other}")),
    }
}

/// Executes a [`figstudio_mcp::live::LiveMutation`] on `tab` when the desktop app hosts the
/// live MCP server.
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub fn handle_live_mutation(
    tabs: &mut [Tab],
    tab_id: u64,
    mutation: figstudio_mcp::live::LiveMutation,
) -> anyhow::Result<serde_json::Value> {
    use figstudio_mcp::live::LiveMutation as M;
    use serde_json::json;

    let tab = tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .ok_or_else(|| anyhow::anyhow!("tab closed"))?;

    match mutation {
        M::ApplyEdits {
            ops,
            label,
            dry_run,
        } => {
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (report, _) = execute_edit_ops(
                doc_mut,
                &mut tab.history,
                &ops,
                label.as_deref(),
                None,
                dry_run,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            if report.applied {
                tab.dirty_unsaved = true;
                tab.motion_preview = None;
            }
            Ok(serde_json::to_value(report)?)
        }
        M::Undo => {
            let undone = undo_tab(tab).map_err(anyhow::Error::msg)?;
            Ok(json!({
                "undone": undone,
                "revision": tab.doc.revision,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            }))
        }
        M::Redo => {
            let redone = redo_tab(tab).map_err(anyhow::Error::msg)?;
            Ok(json!({
                "redone": redone,
                "revision": tab.doc.revision,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            }))
        }
        M::GetHistory => Ok(json!({
            "revision": tab.doc.revision,
            "undo": tab.history.undo_labels(),
            "redo": tab.history.redo_labels(),
        })),
        M::ImportImage { bytes } => {
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (hash, _) =
                figstudio_fig::edit::execute_import_image(doc_mut, &mut tab.history, bytes)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            tab.dirty_unsaved = true;
            Ok(json!({ "sha1Hex": hash, "revision": doc_mut.revision }))
        }
        M::PasteHtml {
            html,
            parent,
            offset,
        } => {
            let target_parent = parent.unwrap_or(tab.page);
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (roots, _) =
                execute_paste_html(doc_mut, &mut tab.history, &html, target_parent, offset)
                    .map_err(|e| anyhow::anyhow!("{e}"))?;
            tab.dirty_unsaved = true;
            if let Some(&first) = roots.first() {
                tab.select(Some(first));
            }
            Ok(json!({
                "pastedRootIds": roots.into_iter().map(|g| g.to_string()).collect::<Vec<_>>(),
                "revision": tab.doc.revision,
            }))
        }
        M::SaveCopy { out_path } => {
            let lazy_src = tab.real_path.exists().then_some(tab.real_path.as_path());
            tab.doc
                .save_with_lazy_source(lazy_src, &out_path)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(json!({
                "savedPath": out_path,
                "revision": tab.doc.revision,
                "nodes": tab.doc.nodes.len(),
            }))
        }
    }
}
