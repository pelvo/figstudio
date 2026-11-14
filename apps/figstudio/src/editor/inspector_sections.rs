//! Auto-Layout, typography, and component action subsections of the Properties Inspector.

use egui::{DragValue, Ui};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Guid, Node, Val};

use crate::tab::Tab;

use super::{apply_tab_ops, local_xywh};

pub(super) fn autolayout_text_and_component_controls(
    ui: &mut Ui,
    tab: &mut Tab,
    sel: Guid,
    node_snapshot: &Node,
) {
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
}
