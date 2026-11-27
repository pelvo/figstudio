//! Accessible bidirectional Properties Inspector (`Edit Layer` collapsible section & history bar).

use egui::{DragValue, Ui};
use figstudio_fig::Val;
use figstudio_fig::edit::EditOp;

use crate::tab::Tab;
use crate::theme;

use super::inspector_sections::autolayout_text_and_component_controls;
use super::{apply_tab_ops_ui, local_xywh, record_tab_error, redo_tab, undo_tab};

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
            let res = undo_tab(tab);
            let _ = record_tab_error(tab, res);
        }
        let r_btn = ui.add_enabled(can_r, egui::Button::new("↷ Redo"));
        crate::a11y::author(&r_btn, "editor.redo");
        if r_btn.clicked() {
            let res = redo_tab(tab);
            let _ = record_tab_error(tab, res);
        }
        if tab.dirty_unsaved {
            ui.colored_label(p.text_dim, format!("• rev {}", tab.doc.revision));
        }
    });
}

/// Renders the interactive Edit controls section inside the right Properties panel.
pub fn properties_edit_controls(ui: &mut Ui, tab: &mut Tab) {
    // One undo step per edit session: when the focused field or the dragged control changes
    // (to another one or to none), or the selection/inner path does, the last coalesced entry is
    // sealed. While a canvas drag is active, `canvas_gizmos` owns sealing it on release so the
    // Properties panel (drawn before the canvas) does not split the drag on its release frame.
    if tab.edit_drag.is_none() {
        let inner_keys = tab
            .inner
            .as_ref()
            .map(figstudio_fig::resolve::InstancePath::keys);
        let focused = ui.ctx().memory(|m| m.focused());
        let dragged = ui.ctx().dragged_id();
        let session = (focused.is_some() || dragged.is_some())
            .then(|| egui::Id::new((focused, dragged, tab.id, tab.selection, inner_keys)));
        if tab.edit_session != session {
            tab.history.seal_coalesce();
            tab.edit_session = session;
        }
    }
    properties_history_bar(ui, tab);

    // Inner-instance text override quick editor when an inner node inside an INSTANCE is selected.
    if let (Some(inst_id), Some(inner)) = (tab.selection, tab.inner.clone())
        && let Ok(resolved) = tab.pick_cache.with(&tab.doc, |_, defs| {
            figstudio_fig::resolve::resolve_path_with(&tab.doc, defs, &inner)
        })
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
        let resp = ui.add(egui::TextEdit::multiline(&mut text_buf).id_salt((
            "inst-text",
            tab.id,
            inst_id,
            inner.keys(),
        )));
        crate::a11y::name_field(&resp, "Instance text override");
        crate::a11y::author(&resp, "editor.instance-text");
        if resp.changed() {
            let ov_path = inner.keys().to_vec();
            let key = format!("gui:inst:text:{inst_id}:{ov_path:?}");
            let _ = apply_tab_ops_ui(
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
                Some(&key),
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
            ui.push_id((tab.id, sel), |ui| {
                // 1. Name, Visibility, Duplicate, Delete
                let mut name_buf = node_snapshot.name.clone();
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let resp = ui.text_edit_singleline(&mut name_buf);
                    crate::a11y::name_field(&resp, "Layer name");
                    crate::a11y::author(&resp, "editor.layer-name");
                    if resp.changed() {
                        let _ = apply_tab_ops_ui(
                            tab,
                            &[EditOp::Rename {
                                target: sel,
                                name: name_buf,
                            }],
                            Some("Rename layer"),
                            Some(&format!("gui:rename:{sel}")),
                        );
                    }
                });

                let mut vis = node_snapshot.data.flag("visible") != Some(false);
                let mut locked = node_snapshot.data.flag("locked") == Some(true);
                ui.horizontal(|ui| {
                    let r_vis = ui.checkbox(&mut vis, "Visible");
                    crate::a11y::author(&r_vis, "editor.visible");
                    if r_vis.changed() {
                        let _ = apply_tab_ops_ui(
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
                        let _ = apply_tab_ops_ui(
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
                        && let Some(ids) = apply_tab_ops_ui(
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
                    if r_del.clicked()
                        && apply_tab_ops_ui(
                            tab,
                            &[EditOp::DeleteNode { target: sel }],
                            Some("Delete"),
                            None,
                        )
                        .is_some()
                    {
                        tab.select(None);
                    }
                });
                if tab.selection != Some(sel) {
                    return;
                }

                // 2. X, Y, W, H, Rotation, Corner Radius
                let (mut x, mut y, mut w, mut h) = local_xywh(&node_snapshot.data);
                let mut x_changed = false;
                let mut y_changed = false;
                let mut w_changed = false;
                let mut h_changed = false;
                ui.horizontal(|ui| {
                    let rx = ui.add(DragValue::new(&mut x).prefix("X ").speed(1.0));
                    crate::a11y::name_field(&rx, "Position X");
                    crate::a11y::author(&rx, "editor.pos-x");
                    x_changed |= rx.changed();
                    let ry = ui.add(DragValue::new(&mut y).prefix("Y ").speed(1.0));
                    crate::a11y::name_field(&ry, "Position Y");
                    crate::a11y::author(&ry, "editor.pos-y");
                    y_changed |= ry.changed();
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
                    w_changed |= rw.changed();
                    let rh = ui.add(
                        DragValue::new(&mut h)
                            .prefix("H ")
                            .speed(1.0)
                            .range(0.0..=100_000.0),
                    );
                    crate::a11y::name_field(&rh, "Height");
                    crate::a11y::author(&rh, "editor.height");
                    h_changed |= rh.changed();
                });
                if x_changed || y_changed {
                    let key = match (x_changed, y_changed) {
                        (true, false) => format!("gui:inspector:pos-x:{sel}"),
                        (false, true) => format!("gui:inspector:pos-y:{sel}"),
                        _ => format!("gui:inspector:pos:{sel}"),
                    };
                    let _ = apply_tab_ops_ui(
                        tab,
                        &[EditOp::Move {
                            target: sel,
                            x,
                            y,
                            relative: false,
                        }],
                        Some("Set position"),
                        Some(&key),
                    );
                }
                if w_changed || h_changed {
                    let key = match (w_changed, h_changed) {
                        (true, false) => format!("gui:inspector:size-w:{sel}"),
                        (false, true) => format!("gui:inspector:size-h:{sel}"),
                        _ => format!("gui:inspector:size:{sel}"),
                    };
                    let _ = apply_tab_ops_ui(
                        tab,
                        &[EditOp::Resize {
                            target: sel,
                            width: w,
                            height: h,
                        }],
                        Some("Set size"),
                        Some(&key),
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
                        let key = format!("gui:inspector:radius:{sel}");
                        let _ = apply_tab_ops_ui(
                            tab,
                            &[EditOp::SetCornerRadii {
                                target: sel,
                                radii: [radius; 4],
                                smoothing: None,
                            }],
                            Some("Set corner radius"),
                            Some(&key),
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
                        let key = format!("gui:inspector:opacity:{sel}");
                        let _ = apply_tab_ops_ui(
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
                            Some(&key),
                        );
                    }
                });

                // 3. Solid Fill Hex Input
                let cur_rgba =
                    first_solid_srgba(&node_snapshot.data).unwrap_or([217, 217, 217, 255]);
                // While the field has focus its text is a draft, keyed by tab, node, and document
                // revision so partial edits do not leak across tabs or outlive external changes.
                let field_id = ui.make_persistent_id(("fill-hex", tab.id, sel));
                let draft_id = egui::Id::new(("fill-hex-draft", tab.id, sel));
                let mut hex_buf = if ui.ctx().memory(|m| m.has_focus(field_id)) {
                    ui.data(|d| d.get_temp::<(u64, String)>(draft_id))
                        .filter(|(rev, _)| *rev == tab.doc.revision)
                        .map(|(_, s)| s)
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    format!("#{:02X}{:02X}{:02X}", cur_rgba[0], cur_rgba[1], cur_rgba[2])
                });
                ui.horizontal(|ui| {
                    ui.label("Fill");
                    let rf = ui.add(
                        egui::TextEdit::singleline(&mut hex_buf)
                            .id(field_id)
                            .desired_width(78.0),
                    );
                    crate::a11y::name_field(&rf, "Fill colour hex");
                    crate::a11y::author(&rf, "editor.fill-hex");
                    if rf.changed() && hex_buf.trim().len() >= 7 {
                        let key = format!("gui:inspector:fill:{sel}");
                        let _ = apply_tab_ops_ui(
                            tab,
                            &[EditOp::SetSolidFill {
                                target: sel,
                                hex: Some(hex_buf.clone()),
                            }],
                            Some("Set fill colour"),
                            Some(&key),
                        );
                    }
                    if rf.has_focus() {
                        ui.data_mut(|d| {
                            d.insert_temp(draft_id, (tab.doc.revision, hex_buf.clone()));
                        });
                    } else {
                        ui.data_mut(|d| d.remove::<(u64, String)>(draft_id));
                    }
                    let r_cf = ui.small_button("Clear Fill");
                    crate::a11y::author(&r_cf, "editor.clear-fill");
                    if r_cf.clicked() {
                        ui.data_mut(|d| d.remove::<(u64, String)>(draft_id));
                        let _ = apply_tab_ops_ui(
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

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod tests;
