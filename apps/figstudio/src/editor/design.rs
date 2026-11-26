//! The Design panel's editable sections for the selection: Position, Layout, Appearance,
//! Fill, Stroke and Effects. Every control builds `EditOp`s for each selected layer as one
//! undo step; a value the layers do not share shows `Mixed`.

use egui::Ui;
use figstudio_fig::edit::{
    AlignEdge, Axis, EditOp, EffectPatch, EffectSpec, PaintPatch, PaintSlot, PaintSpec, Pivot,
};
use figstudio_fig::{Guid, Val};

use crate::panels::{section, section_body, section_title};
use crate::tab::Tab;

use super::apply_tab_ops;
use super::commands::{self, EditCmd};
use super::widgets::{ColourChange, button, colour_row, num_field, rgba_of, shared, toggle};

/// Figma's blend modes in its menu order (`PASS_THROUGH` only for containers).
const BLENDS: [(&str, &str); 17] = [
    ("PASS_THROUGH", "Pass through"),
    ("NORMAL", "Normal"),
    ("DARKEN", "Darken"),
    ("MULTIPLY", "Multiply"),
    ("COLOR_BURN", "Color burn"),
    ("LIGHTEN", "Lighten"),
    ("SCREEN", "Screen"),
    ("COLOR_DODGE", "Color dodge"),
    ("OVERLAY", "Overlay"),
    ("SOFT_LIGHT", "Soft light"),
    ("HARD_LIGHT", "Hard light"),
    ("DIFFERENCE", "Difference"),
    ("EXCLUSION", "Exclusion"),
    ("HUE", "Hue"),
    ("SATURATION", "Saturation"),
    ("COLOR", "Color"),
    ("LUMINOSITY", "Luminosity"),
];

/// The layers the panel edits: the selection, pages left out.
fn targets(tab: &Tab) -> Vec<Guid> {
    tab.selected()
        .into_iter()
        .filter(|&id| {
            tab.doc
                .node(id)
                .is_some_and(|n| n.kind != "CANVAS" && n.kind != "DOCUMENT")
        })
        .collect()
}

/// A number from each target's data.
fn nums(tab: &Tab, ids: &[Guid], f: impl Fn(&Val) -> Option<f64>) -> Vec<f64> {
    ids.iter()
        .filter_map(|id| tab.doc.node(*id).map(|n| f(&n.data).unwrap_or(0.0)))
        .collect()
}

fn num(data: &Val, path: &[&str]) -> Option<f64> {
    data.path(path).and_then(Val::as_f32).map(f64::from)
}

/// Rotation as Figma shows it: degrees counter-clockwise.
fn rotation(data: &Val) -> f64 {
    let t = figstudio_fig::edit::space::Xform::of(data);
    (-t.m10).atan2(t.m00).to_degrees()
}

/// Runs one op per target as a single undo step. `key` coalesces scrubbing into one step.
fn each(
    tab: &mut Tab,
    ids: &[Guid],
    label: &str,
    key: Option<&str>,
    op: impl Fn(Guid, &Val) -> Option<EditOp>,
) {
    let ops: Vec<EditOp> = ids
        .iter()
        .filter_map(|&id| tab.doc.node(id).and_then(|n| op(id, &n.data)))
        .collect();
    if !ops.is_empty() {
        let _ = apply_tab_ops(tab, &ops, Some(label), key);
    }
}

/// One layer setting changed: opacity, blend mode, clip content or aspect lock.
#[derive(Clone)]
enum Prop {
    Opacity(f64),
    Blend(String),
    Clip(bool),
    AspectLock(bool),
}

fn layer_props(target: Guid, prop: Prop) -> EditOp {
    let (mut opacity, mut blend_mode, mut clip_content, mut proportions_locked) =
        (None, None, None, None);
    match prop {
        Prop::Opacity(v) => opacity = Some(v),
        Prop::Blend(b) => blend_mode = Some(b),
        Prop::Clip(c) => clip_content = Some(c),
        Prop::AspectLock(l) => proportions_locked = Some(l),
    }
    EditOp::SetLayerProps {
        target,
        visible: None,
        locked: None,
        opacity,
        blend_mode,
        clip_content,
        proportions_locked,
    }
}

/// The Design panel for the selection.
pub fn design_sections(ui: &mut Ui, tab: &mut Tab) {
    let ids = targets(tab);
    if ids.is_empty() {
        return;
    }
    // Figma's order, with the layer-type sections after the geometry.
    super::design_more::name(ui, tab, &ids);
    position(ui, tab, &ids);
    layout(ui, tab, &ids);
    appearance(ui, tab, &ids);
    super::design_proto::boolean_mask(ui, tab, &ids);
    super::design_more::kind_sections(ui, tab, &ids);
    paints(ui, tab, &ids, PaintSlot::Fill);
    paints(ui, tab, &ids, PaintSlot::Stroke);
    effects(ui, tab, &ids);
    super::design_more::export(ui, tab, &ids);
    super::design_proto::prototype(ui, tab, &ids);
}

fn position(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Position", |ui| {
        ui.horizontal(|ui| {
            for (text, edge, name) in [
                ("⇤", AlignEdge::Left, "Align left"),
                ("⇹", AlignEdge::HCenter, "Align horizontal centers"),
                ("⇥", AlignEdge::Right, "Align right"),
                ("⤒", AlignEdge::Top, "Align top"),
                ("⇕", AlignEdge::VCenter, "Align vertical centers"),
                ("⤓", AlignEdge::Bottom, "Align bottom"),
            ] {
                let cmd = EditCmd::Align(edge);
                let r = ui.add_enabled_ui(cmd.enabled(tab), |ui| button(ui, text, name));
                if r.inner.clicked() {
                    let _ = commands::run(tab, cmd);
                }
            }
        });
        let x = shared(&nums(tab, ids, |d| num(d, &["transform", "m02"])));
        let y = shared(&nums(tab, ids, |d| num(d, &["transform", "m12"])));
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, "X", "X position", x, 1.0, 56.0) {
                each(tab, ids, "Move layer", Some("design:x"), |id, d| {
                    Some(EditOp::Move {
                        target: id,
                        x: v,
                        y: num(d, &["transform", "m12"]).unwrap_or(0.0),
                        relative: false,
                    })
                });
            }
            if let Some(v) = num_field(ui, "Y", "Y position", y, 1.0, 56.0) {
                each(tab, ids, "Move layer", Some("design:y"), |id, d| {
                    Some(EditOp::Move {
                        target: id,
                        x: num(d, &["transform", "m02"]).unwrap_or(0.0),
                        y: v,
                        relative: false,
                    })
                });
            }
        });
        let r = shared(&nums(tab, ids, |d| Some(rotation(d))));
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, "↻", "Layer rotation", r.map(|v| v.round()), 1.0, 44.0)
            {
                each(
                    tab,
                    ids,
                    "Rotate layer",
                    Some("design:rotation"),
                    |id, _| {
                        Some(EditOp::Rotate {
                            target: id,
                            degrees: -v,
                            pivot: Pivot::Center,
                        })
                    },
                );
            }
            for (text, axis, name) in [
                ("⇋", Axis::Horizontal, "Flip horizontal"),
                ("⇅", Axis::Vertical, "Flip vertical"),
            ] {
                if button(ui, text, name).clicked() {
                    let _ = commands::run(tab, EditCmd::Flip(axis));
                }
            }
        });
    });
}

fn layout(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Layout", |ui| {
        let w = shared(&nums(tab, ids, |d| num(d, &["size", "x"])));
        let h = shared(&nums(tab, ids, |d| num(d, &["size", "y"])));
        let locked = ids.iter().all(|id| {
            tab.doc
                .node(*id)
                .is_some_and(|n| n.data.flag("proportionsConstrained") == Some(true))
        });
        ui.horizontal(|ui| {
            let new_w = num_field(ui, "W", "Width", w, 1.0, 56.0);
            let new_h = num_field(ui, "H", "Height", h, 1.0, 56.0);
            if new_w.is_some() || new_h.is_some() {
                each(tab, ids, "Resize layer", Some("design:size"), |id, d| {
                    let (cw, ch) = (
                        num(d, &["size", "x"]).unwrap_or(0.0),
                        num(d, &["size", "y"]).unwrap_or(0.0),
                    );
                    let (mut nw, mut nh) = (new_w.unwrap_or(cw), new_h.unwrap_or(ch));
                    // The aspect lock keeps each layer's own ratio.
                    if locked && cw > 0.0 && ch > 0.0 {
                        if new_w.is_some() {
                            nh = nw * ch / cw;
                        } else {
                            nw = nh * cw / ch;
                        }
                    }
                    Some(EditOp::Resize {
                        target: id,
                        width: nw.max(0.01),
                        height: nh.max(0.01),
                    })
                });
            }
            if toggle(ui, "🔗", "Lock aspect ratio", locked).clicked() {
                each(tab, ids, "Lock aspect ratio", None, |id, _| {
                    Some(layer_props(id, Prop::AspectLock(!locked)))
                });
            }
        });
        let frames = ids.iter().all(|id| {
            tab.doc
                .node(*id)
                .is_some_and(|n| matches!(n.kind.as_str(), "FRAME" | "SYMBOL" | "INSTANCE"))
        });
        if frames {
            let clips = ids.iter().all(|id| {
                tab.doc
                    .node(*id)
                    .is_some_and(|n| n.data.flag("frameMaskDisabled") != Some(true))
            });
            let mut on = clips;
            let r = ui.checkbox(&mut on, "Clip content");
            crate::a11y::author(&r, "design.clip-content");
            if r.changed() {
                each(tab, ids, "Clip content", None, |id, _| {
                    Some(layer_props(id, Prop::Clip(on)))
                });
            }
        }
    });
}

/// Layers whose corners can be rounded.
fn has_corners(kind: &str) -> bool {
    matches!(
        kind,
        "ROUNDED_RECTANGLE"
            | "RECTANGLE"
            | "FRAME"
            | "SYMBOL"
            | "INSTANCE"
            | "REGULAR_POLYGON"
            | "STAR"
    )
}

fn appearance(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Appearance", |ui| {
        let visible = ids.iter().all(|id| {
            tab.doc
                .node(*id)
                .is_some_and(|n| n.data.flag("visible") != Some(false))
        });
        let opacity = shared(&nums(tab, ids, |d| {
            Some(d.num("opacity").map_or(1.0, f64::from) * 100.0)
        }));
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, "◐", "Opacity", opacity.map(f64::round), 1.0, 40.0) {
                let v = (v / 100.0).clamp(0.0, 1.0);
                each(tab, ids, "Set opacity", Some("design:opacity"), |id, _| {
                    Some(layer_props(id, Prop::Opacity(v)))
                });
            }
            if toggle(ui, if visible { "👁" } else { "–" }, "Visible", visible).clicked() {
                let _ = commands::run(tab, EditCmd::ToggleVisible);
            }
        });
        let blend = {
            let modes: Vec<&str> = ids
                .iter()
                .filter_map(|id| tab.doc.node(*id))
                .map(|n| n.data.text("blendMode").unwrap_or("PASS_THROUGH"))
                .collect();
            modes
                .first()
                .filter(|m| modes.iter().all(|x| x == *m))
                .map(|m| m.to_string())
        };
        let shown = blend
            .as_deref()
            .and_then(|b| BLENDS.iter().find(|(k, _)| *k == b))
            .map_or("Mixed", |(_, l)| *l);
        let mut picked = None;
        let combo = egui::ComboBox::from_id_salt("design-blend")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (k, l) in BLENDS {
                    if ui
                        .selectable_label(blend.as_deref() == Some(k), l)
                        .clicked()
                    {
                        picked = Some(k);
                    }
                }
            });
        crate::a11y::name_field(&combo.response, "Blend mode");
        crate::a11y::author(&combo.response, "design.blend-mode");
        if let Some(k) = picked {
            each(tab, ids, "Set blend mode", None, |id, _| {
                Some(layer_props(id, Prop::Blend(k.to_string())))
            });
        }
        let corner_ids: Vec<Guid> = ids
            .iter()
            .copied()
            .filter(|id| tab.doc.node(*id).is_some_and(|n| has_corners(&n.kind)))
            .collect();
        if corner_ids.len() == ids.len() {
            corners(ui, tab, &corner_ids);
        }
    });
}

fn corner_values(d: &Val) -> [f64; 4] {
    let all = d.num("cornerRadius").map_or(0.0, f64::from);
    if d.flag("rectangleCornerRadiiIndependent") == Some(true) {
        [
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
        ]
        .map(|k| d.num(k).map_or(all, f64::from))
    } else {
        [all; 4]
    }
}

fn corners(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let sets: Vec<[f64; 4]> = ids
        .iter()
        .filter_map(|id| tab.doc.node(*id).map(|n| corner_values(&n.data)))
        .collect();
    let independent_id = egui::Id::new(("design-corners-split", ids.first().copied()));
    let differ = sets
        .iter()
        .any(|c| c.iter().any(|v| (v - c[0]).abs() > 1e-6));
    let mut split: bool = ui.data(|d| d.get_temp(independent_id)).unwrap_or(differ);
    let set_all = |tab: &mut Tab, radii: &dyn Fn([f64; 4]) -> [f64; 4]| {
        each(
            tab,
            ids,
            "Set corner radius",
            Some("design:radius"),
            |id, d| {
                let r = radii(corner_values(d));
                Some(EditOp::SetCornerRadii {
                    target: id,
                    radii: r.map(|v| v.max(0.0) as f32),
                    smoothing: None,
                })
            },
        );
    };
    ui.horizontal(|ui| {
        if split {
            for (i, (label, name)) in [
                ("┌", "Top left radius"),
                ("┐", "Top right radius"),
                ("┘", "Bottom right radius"),
                ("└", "Bottom left radius"),
            ]
            .into_iter()
            .enumerate()
            {
                let v = shared(&sets.iter().map(|c| c[i]).collect::<Vec<_>>());
                if let Some(nv) = num_field(ui, label, name, v, 0.5, 30.0) {
                    set_all(tab, &|mut c| {
                        c[i] = nv;
                        c
                    });
                }
            }
        } else {
            let v = shared(&sets.iter().map(|c| c[0]).collect::<Vec<_>>());
            if let Some(nv) = num_field(ui, "◜", "Corner radius", v, 0.5, 44.0) {
                set_all(tab, &|_| [nv; 4]);
            }
        }
        if toggle(ui, "⛶", "Independent corners", split).clicked() {
            split = !split;
            ui.data_mut(|d| d.insert_temp(independent_id, split));
        }
    });
}

fn slot_name(slot: PaintSlot) -> &'static str {
    match slot {
        PaintSlot::Fill => "Fill",
        PaintSlot::Stroke => "Stroke",
    }
}

/// Each target's paint list for `slot`.
fn paint_lists(tab: &Tab, ids: &[Guid], slot: PaintSlot) -> Vec<Vec<Val>> {
    ids.iter()
        .filter_map(|id| tab.doc.node(*id))
        .map(|n| {
            n.data
                .get(slot.field())
                .map(|v| v.items().iter().map(|p| p.clone().into_owned()).collect())
                .unwrap_or_default()
        })
        .collect()
}

fn same_lists(lists: &[Vec<Val>]) -> bool {
    lists.first().is_some_and(|f| lists.iter().all(|l| l == f))
}

fn paints(ui: &mut Ui, tab: &mut Tab, ids: &[Guid], slot: PaintSlot) {
    let lists = paint_lists(tab, ids, slot);
    let name = slot_name(slot);
    // An empty list shows only its title and +, as in Figma.
    let empty = lists.iter().all(Vec::is_empty);
    if section_title(ui, name, Some(&format!("Add {}", name.to_lowercase()))) {
        let hex = if slot == PaintSlot::Fill {
            "#D9D9D9"
        } else {
            "#000000"
        };
        each(
            tab,
            ids,
            &format!("Add {}", name.to_lowercase()),
            None,
            |id, _| {
                Some(EditOp::AddPaint {
                    target: id,
                    slot,
                    paint: PaintSpec::Solid {
                        hex: hex.into(),
                        opacity: None,
                        visible: None,
                        blend_mode: None,
                    },
                    index: None,
                })
            },
        );
        return;
    }
    if empty {
        ui.add_space(6.0);
        return;
    }
    section_body(ui, |ui| {
        if !same_lists(&lists) {
            ui.label(egui::RichText::new("Mixed").color(crate::theme::of(ui).text_faint));
            return;
        }
        // Figma lists the top paint first; the file stores it last.
        let list = &lists[0];
        for index in (0..list.len()).rev() {
            paint_row(ui, tab, ids, slot, index, &list[index], list.len());
        }
        if slot == PaintSlot::Stroke {
            stroke_style(ui, tab, ids);
        }
    });
}

fn paint_row(
    ui: &mut Ui,
    tab: &mut Tab,
    ids: &[Guid],
    slot: PaintSlot,
    index: usize,
    paint: &Val,
    count: usize,
) {
    let kind = paint.text("type").unwrap_or("SOLID");
    let visible = paint.flag("visible") != Some(false);
    let opacity = paint.num("opacity").map_or(1.0, f64::from);
    // The first row keeps the plain label so screen readers and tests find "Fill colour hex".
    let label = if index + 1 == count {
        slot_name(slot).to_string()
    } else {
        format!("{} {}", slot_name(slot), count - index)
    };
    let update = |tab: &mut Tab, patch: PaintPatch, key: Option<&str>| {
        each(
            tab,
            ids,
            &format!("Edit {}", slot_name(slot).to_lowercase()),
            key,
            |id, _| {
                Some(EditOp::UpdatePaint {
                    target: id,
                    slot,
                    index,
                    patch: patch.clone(),
                })
            },
        );
    };
    ui.horizontal(|ui| {
        if kind == "SOLID" {
            let rgba = rgba_of(paint.get("color"), opacity);
            match colour_row(ui, &label, rgba) {
                Some(ColourChange::Hex(hex)) => update(
                    tab,
                    PaintPatch {
                        hex: Some(hex),
                        ..Default::default()
                    },
                    Some(&format!("design:{label}:hex")),
                ),
                Some(ColourChange::Opacity(o)) => update(
                    tab,
                    PaintPatch {
                        opacity: Some(o),
                        ..Default::default()
                    },
                    Some(&format!("design:{label}:opacity")),
                ),
                None => {}
            }
        } else {
            let what = match kind {
                "GRADIENT_LINEAR" => "Linear gradient",
                "GRADIENT_RADIAL" => "Radial gradient",
                "GRADIENT_ANGULAR" => "Angular gradient",
                "GRADIENT_DIAMOND" => "Diamond gradient",
                "IMAGE" => "Image",
                "VIDEO" => "Video",
                _ => "Paint",
            };
            ui.label(what);
            if let Some(v) = num_field(
                ui,
                "%",
                &format!("{label} opacity"),
                Some((opacity * 100.0).round()),
                1.0,
                34.0,
            ) {
                update(
                    tab,
                    PaintPatch {
                        opacity: Some((v / 100.0).clamp(0.0, 1.0)),
                        ..Default::default()
                    },
                    Some(&format!("design:{label}:opacity")),
                );
            }
        }
        if toggle(
            ui,
            if visible { "👁" } else { "–" },
            &format!("{label} visible"),
            visible,
        )
        .clicked()
        {
            update(
                tab,
                PaintPatch {
                    visible: Some(!visible),
                    ..Default::default()
                },
                None,
            );
        }
        if button(ui, "−", &format!("Remove {}", label.to_lowercase())).clicked() {
            each(
                tab,
                ids,
                &format!("Remove {}", slot_name(slot).to_lowercase()),
                None,
                |id, _| {
                    Some(EditOp::RemovePaint {
                        target: id,
                        slot,
                        index,
                    })
                },
            );
        }
    });
}

fn stroke_style(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let weight = shared(&nums(tab, ids, |d| d.num("strokeWeight").map(f64::from)));
    let aligns: Vec<String> = ids
        .iter()
        .filter_map(|id| tab.doc.node(*id))
        .map(|n| n.data.text("strokeAlign").unwrap_or("CENTER").to_string())
        .collect();
    let align = aligns
        .first()
        .filter(|a| aligns.iter().all(|x| x == *a))
        .cloned();
    let style = |target, weight: Option<f64>, align: Option<String>| EditOp::SetStrokeStyle {
        target,
        weight,
        align,
        cap: None,
        join: None,
        miter_limit: None,
        dashes: None,
        sides: None,
    };
    ui.horizontal(|ui| {
        let mut picked = None;
        let shown = match align.as_deref() {
            Some("INSIDE") => "Inside",
            Some("OUTSIDE") => "Outside",
            Some(_) => "Center",
            None => "Mixed",
        };
        let combo = egui::ComboBox::from_id_salt("design-stroke-align")
            .selected_text(shown)
            .width(80.0)
            .show_ui(ui, |ui| {
                for (k, l) in [
                    ("INSIDE", "Inside"),
                    ("CENTER", "Center"),
                    ("OUTSIDE", "Outside"),
                ] {
                    if ui
                        .selectable_label(align.as_deref() == Some(k), l)
                        .clicked()
                    {
                        picked = Some(k);
                    }
                }
            });
        crate::a11y::name_field(&combo.response, "Stroke position");
        crate::a11y::author(&combo.response, "design.stroke-position");
        if let Some(k) = picked {
            each(tab, ids, "Stroke position", None, |id, _| {
                Some(style(id, None, Some(k.to_string())))
            });
        }
        if let Some(v) = num_field(ui, "≡", "Stroke weight", weight, 0.5, 40.0) {
            each(
                tab,
                ids,
                "Stroke weight",
                Some("design:stroke-weight"),
                |id, _| Some(style(id, Some(v.max(0.0)), None)),
            );
        }
    });
}

fn effects(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let lists: Vec<Vec<Val>> = ids
        .iter()
        .filter_map(|id| tab.doc.node(*id))
        .map(|n| {
            n.data
                .get("effects")
                .map(|v| v.items().iter().map(|p| p.clone().into_owned()).collect())
                .unwrap_or_default()
        })
        .collect();
    if section_title(ui, "Effects", Some("Add effect")) {
        each(tab, ids, "Add effect", None, |id, _| {
            Some(EditOp::AddEffect {
                target: id,
                effect: EffectSpec::DropShadow {
                    hex: None,
                    offset: None,
                    radius: None,
                    spread: None,
                    show_behind: None,
                    visible: None,
                    blend_mode: None,
                },
                index: None,
            })
        });
        return;
    }
    if lists.iter().all(Vec::is_empty) {
        ui.add_space(6.0);
        return;
    }
    section_body(ui, |ui| {
        if !same_lists(&lists) {
            ui.label(egui::RichText::new("Mixed").color(crate::theme::of(ui).text_faint));
            return;
        }
        for (index, effect) in lists[0].iter().enumerate() {
            effect_row(ui, tab, ids, index, effect);
        }
    });
}

fn effect_row(ui: &mut Ui, tab: &mut Tab, ids: &[Guid], index: usize, effect: &Val) {
    let kind = effect.text("type").unwrap_or("DROP_SHADOW");
    let visible = effect.flag("visible") != Some(false);
    let name = match kind {
        "DROP_SHADOW" => "Drop shadow",
        "INNER_SHADOW" => "Inner shadow",
        "FOREGROUND_BLUR" => "Layer blur",
        "BACKGROUND_BLUR" => "Background blur",
        _ => "Effect",
    };
    let label = format!("{name} {}", index + 1);
    let update = |tab: &mut Tab, patch: EffectPatch, key: Option<&str>| {
        each(tab, ids, "Edit effect", key, |id, _| {
            Some(EditOp::UpdateEffect {
                target: id,
                index,
                patch: patch.clone(),
            })
        });
    };
    ui.horizontal(|ui| {
        ui.label(name);
        if let Some(v) = num_field(
            ui,
            "◌",
            &format!("{label} blur"),
            effect.num("radius").map(f64::from),
            0.5,
            34.0,
        ) {
            update(
                tab,
                EffectPatch {
                    radius: Some(v.max(0.0)),
                    ..Default::default()
                },
                Some(&format!("design:{label}:radius")),
            );
        }
        if toggle(
            ui,
            if visible { "👁" } else { "–" },
            &format!("{label} visible"),
            visible,
        )
        .clicked()
        {
            update(
                tab,
                EffectPatch {
                    visible: Some(!visible),
                    ..Default::default()
                },
                None,
            );
        }
        if button(ui, "−", &format!("Remove {}", label.to_lowercase())).clicked() {
            each(tab, ids, "Remove effect", None, |id, _| {
                Some(EditOp::RemoveEffect { target: id, index })
            });
        }
    });
    if kind.ends_with("_SHADOW") {
        let ox = effect
            .path(&["offset", "x"])
            .and_then(Val::as_f32)
            .map(f64::from);
        let oy = effect
            .path(&["offset", "y"])
            .and_then(Val::as_f32)
            .map(f64::from);
        ui.horizontal(|ui| {
            let nx = num_field(ui, "X", &format!("{label} x"), ox, 0.5, 34.0);
            let ny = num_field(ui, "Y", &format!("{label} y"), oy, 0.5, 34.0);
            if nx.is_some() || ny.is_some() {
                update(
                    tab,
                    EffectPatch {
                        offset: Some([nx.or(ox).unwrap_or(0.0), ny.or(oy).unwrap_or(0.0)]),
                        ..Default::default()
                    },
                    Some(&format!("design:{label}:offset")),
                );
            }
            if let Some(v) = num_field(
                ui,
                "S",
                &format!("{label} spread"),
                effect.num("spread").map(f64::from),
                0.5,
                34.0,
            ) {
                update(
                    tab,
                    EffectPatch {
                        spread: Some(v),
                        ..Default::default()
                    },
                    Some(&format!("design:{label}:spread")),
                );
            }
        });
        let alpha = effect
            .path(&["color", "a"])
            .and_then(Val::as_f32)
            .map_or(1.0, f64::from);
        ui.horizontal(
            |ui| match colour_row(ui, &label, rgba_of(effect.get("color"), alpha)) {
                Some(ColourChange::Hex(hex)) => {
                    let a = (alpha * 255.0).round() as u8;
                    update(
                        tab,
                        EffectPatch {
                            hex: Some(format!("{hex}{a:02X}")),
                            ..Default::default()
                        },
                        Some(&format!("design:{label}:hex")),
                    );
                }
                Some(ColourChange::Opacity(o)) => {
                    let rgb = super::widgets::hex_of(rgba_of(effect.get("color"), 1.0));
                    update(
                        tab,
                        EffectPatch {
                            hex: Some(format!("{rgb}{:02X}", (o * 255.0).round() as u8)),
                            ..Default::default()
                        },
                        Some(&format!("design:{label}:hex")),
                    );
                }
                None => {}
            },
        );
    }
}
