//! Accessible bidirectional Properties Inspector (`Edit Layer` collapsible section & history bar).

use egui::{DragValue, Ui};
use figstudio_fig::Val;
use figstudio_fig::edit::EditOp;

use crate::tab::Tab;
use crate::theme;

use super::inspector_sections::autolayout_text_and_component_controls;
use super::{apply_tab_ops, local_xywh, redo_tab, undo_tab};

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

            autolayout_text_and_component_controls(ui, tab, sel, &node_snapshot);
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
