//! Auto-Layout, typography, and component action subsections of the Properties Inspector.

use egui::{DragValue, Ui};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Guid, Node, Val};

use crate::tab::Tab;

use super::{apply_tab_ops_ui, local_xywh};

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
        let orig_gap = node_snapshot.data.num("stackSpacing").map(f64::from);
        let orig_left = node_snapshot
            .data
            .num("stackHorizontalPadding")
            .map(f64::from);
        let orig_top = node_snapshot
            .data
            .num("stackVerticalPadding")
            .map(f64::from);
        let orig_right = node_snapshot.data.num("stackPaddingRight").map(f64::from);
        let orig_bottom = node_snapshot.data.num("stackPaddingBottom").map(f64::from);
        let has_any_padding = orig_left.is_some()
            || orig_top.is_some()
            || orig_right.is_some()
            || orig_bottom.is_some();

        let mut gap = orig_gap.unwrap_or(8.0);
        let mut ph = orig_left.unwrap_or(12.0);
        let mut pv = orig_top.unwrap_or(12.0);
        ui.horizontal(|ui| {
            ui.label("Auto-Layout");
            for (lbl, mode, aid) in [
                ("Off", "NONE", "editor.al-off"),
                ("Horiz", "HORIZONTAL", "editor.al-horiz"),
                ("Vert", "VERTICAL", "editor.al-vert"),
            ] {
                let rb = ui.selectable_label(cur_mode == mode, lbl);
                crate::a11y::author(&rb, aid);
                if rb.clicked() && cur_mode != mode {
                    let enabling = mode != "NONE";
                    let _ = apply_tab_ops_ui(
                        tab,
                        &[EditOp::SetAutoLayout {
                            target: sel,
                            mode: mode.into(),
                            spacing: (enabling && orig_gap.is_none()).then_some(gap),
                            counter_spacing: None,
                            padding: (enabling && !has_any_padding).then_some([pv, ph, pv, ph]),
                            primary_sizing: (enabling
                                && node_snapshot.data.text("stackPrimarySizing").is_none())
                            .then(|| "HUG".into()),
                            counter_sizing: (enabling
                                && node_snapshot.data.text("stackCounterSizing").is_none())
                            .then(|| "HUG".into()),
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
            let mut gap_changed = false;
            let mut padx_changed = false;
            let mut pady_changed = false;
            ui.horizontal(|ui| {
                let rg = ui.add(DragValue::new(&mut gap).prefix("Gap ").speed(1.0));
                crate::a11y::name_field(&rg, "Auto-layout gap");
                crate::a11y::author(&rg, "editor.al-gap");
                gap_changed |= rg.changed();
                let rpx = ui.add(DragValue::new(&mut ph).prefix("PadX ").speed(1.0));
                crate::a11y::name_field(&rpx, "Auto-layout horizontal padding");
                crate::a11y::author(&rpx, "editor.al-padx");
                padx_changed |= rpx.changed();
                let rpy = ui.add(DragValue::new(&mut pv).prefix("PadY ").speed(1.0));
                crate::a11y::name_field(&rpy, "Auto-layout vertical padding");
                crate::a11y::author(&rpy, "editor.al-pady");
                pady_changed |= rpy.changed();
            });
            if gap_changed || padx_changed || pady_changed {
                let padding = (padx_changed || pady_changed).then(|| {
                    let cur_left = orig_left.unwrap_or(12.0);
                    let cur_top = orig_top.unwrap_or(12.0);
                    let cur_right = orig_right.unwrap_or(cur_left);
                    let cur_bottom = orig_bottom.unwrap_or(cur_top);
                    let (left, right) = if padx_changed {
                        (ph, ph)
                    } else {
                        (cur_left, cur_right)
                    };
                    let (top, bottom) = if pady_changed {
                        (pv, pv)
                    } else {
                        (cur_top, cur_bottom)
                    };
                    [top, right, bottom, left]
                });
                let key = match (gap_changed, padx_changed, pady_changed) {
                    (true, false, false) => format!("gui:inspector:al-gap:{sel}"),
                    (false, true, false) => format!("gui:inspector:al-padx:{sel}"),
                    (false, false, true) => format!("gui:inspector:al-pady:{sel}"),
                    _ => format!("gui:inspector:al:{sel}"),
                };
                let _ = apply_tab_ops_ui(
                    tab,
                    &[EditOp::SetAutoLayout {
                        target: sel,
                        mode: cur_mode,
                        spacing: gap_changed.then_some(gap),
                        counter_spacing: None,
                        padding,
                        primary_sizing: None,
                        counter_sizing: None,
                        primary_align: None,
                        counter_align: None,
                        wrap: None,
                    }],
                    Some("Adjust Auto-Layout"),
                    Some(&key),
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
            let key = format!("gui:inspector:text:{sel}");
            let _ = apply_tab_ops_ui(
                tab,
                &[EditOp::SetText {
                    target: sel,
                    characters: chars,
                }],
                Some("Edit text"),
                Some(&key),
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
                let key = format!("gui:inspector:fontsize:{sel}");
                let _ = apply_tab_ops_ui(
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
                    Some(&key),
                );
            }
            for (sty, aid) in [
                ("Regular", "editor.weight-regular"),
                ("Bold", "editor.weight-bold"),
            ] {
                let rb = ui.small_button(sty);
                crate::a11y::author(&rb, aid);
                if rb.clicked() {
                    let _ = apply_tab_ops_ui(
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
                let _ = apply_tab_ops_ui(
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
                if let Some(ids) = apply_tab_ops_ui(
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
                let _ = apply_tab_ops_ui(
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
                && let Some(ids) = apply_tab_ops_ui(
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
