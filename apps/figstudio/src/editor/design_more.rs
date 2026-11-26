//! The Design panel's layer-type sections: name, auto layout (and how a child sits in one),
//! constraints, text, instance and export. Like `design.rs`, every control builds `EditOp`s
//! for each selected layer as one undo step.

use egui::Ui;
use figstudio_fig::edit::{EditOp, ExportSpec, TextLength, TypographyExtra};
use figstudio_fig::{Guid, Val};

use crate::panels::{section, section_body, section_title};
use crate::tab::Tab;

use super::apply_tab_ops;
use super::widgets::{button, num_field, num_field_or, shared, toggle};

/// The value every target has for `f`, or `None` when they differ.
fn same<T: PartialEq + Clone>(tab: &Tab, ids: &[Guid], f: impl Fn(&Val) -> T) -> Option<T> {
    let vals: Vec<T> = ids
        .iter()
        .filter_map(|id| tab.doc.node(*id).map(|n| f(&n.data)))
        .collect();
    let first = vals.first()?.clone();
    vals.iter().all(|v| *v == first).then_some(first)
}

fn nums(tab: &Tab, ids: &[Guid], f: impl Fn(&Val) -> f64) -> Option<f64> {
    shared(
        &ids.iter()
            .filter_map(|id| tab.doc.node(*id).map(|n| f(&n.data)))
            .collect::<Vec<_>>(),
    )
}

fn kinds_all(tab: &Tab, ids: &[Guid], kinds: &[&str]) -> bool {
    ids.iter().all(|id| {
        tab.doc
            .node(*id)
            .is_some_and(|n| kinds.contains(&n.kind.as_str()))
    })
}

/// One op per target as one undo step (`key` coalesces scrubbing).
fn each(tab: &mut Tab, ids: &[Guid], label: &str, key: Option<&str>, op: impl Fn(Guid) -> EditOp) {
    let ops: Vec<EditOp> = ids.iter().map(|&id| op(id)).collect();
    if !ops.is_empty() {
        let _ = apply_tab_ops(tab, &ops, Some(label), key);
    }
}

/// A faint axis tag before a combo box.
fn tag(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(crate::theme::of(ui).text_faint));
}

/// A combo box of `(stored, label)` options; returns the stored value picked.
fn combo(
    ui: &mut Ui,
    name: &str,
    current: Option<&str>,
    options: &[(&'static str, &'static str)],
    width: f32,
) -> Option<&'static str> {
    let shown = current
        .and_then(|c| options.iter().find(|(k, _)| *k == c))
        .map_or("Mixed", |(_, l)| *l);
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(("design-combo", name))
        .selected_text(shown)
        .width(width)
        .show_ui(ui, |ui| {
            for (k, l) in options {
                if ui.selectable_label(current == Some(*k), *l).clicked() {
                    picked = Some(*k);
                }
            }
        });
    crate::a11y::name_field(&r.response, name);
    crate::a11y::author(&r.response, format!("design.{}", crate::a11y::slug(name)));
    picked.filter(|p| Some(*p) != current)
}

/// The sections that depend on what the selected layers are: auto layout, how they sit in
/// their parent, text, instance.
pub(super) fn kind_sections(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    if kinds_all(tab, ids, &["FRAME", "SYMBOL", "INSTANCE"]) {
        auto_layout(ui, tab, ids);
    }
    child(ui, tab, ids);
    if kinds_all(tab, ids, &["TEXT"]) {
        text(ui, tab, ids);
    }
    if kinds_all(tab, ids, &["INSTANCE"]) {
        instance(ui, tab, ids);
    }
}

pub(super) fn name(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let [id] = ids else { return };
    let id = *id;
    let current = tab.doc.node(id).map(|n| n.name.clone()).unwrap_or_default();
    let draft_id = egui::Id::new(("design-name", id));
    let mut text: String = ui.data(|d| d.get_temp(draft_id)).unwrap_or(current.clone());
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(f32::INFINITY));
    crate::a11y::name_field(&r, "Layer name");
    crate::a11y::author(&r, "design.layer-name");
    let commit = r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    if r.has_focus() && !commit {
        ui.data_mut(|d| d.insert_temp(draft_id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(draft_id));
    }
    if commit && text.trim() != current && !text.trim().is_empty() {
        let _ = apply_tab_ops(
            tab,
            &[EditOp::Rename {
                target: id,
                name: text.trim().to_string(),
            }],
            Some("Rename layer"),
            None,
        );
    }
    ui.add_space(4.0);
}

fn stack_mode(d: &Val) -> String {
    d.text("stackMode").unwrap_or("NONE").to_string()
}

fn sizing(d: &Val, field: &str) -> &'static str {
    match d.text(field) {
        Some("RESIZE_TO_FIT" | "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE") => "HUG",
        _ => "FIXED",
    }
}

/// `SetAutoLayout` keeping everything but the given changes.
#[derive(Default, Clone)]
struct Stack {
    mode: Option<String>,
    spacing: Option<f64>,
    padding: Option<[Option<f64>; 4]>,
    primary_sizing: Option<&'static str>,
    counter_sizing: Option<&'static str>,
    primary_align: Option<&'static str>,
    counter_align: Option<&'static str>,
    wrap: Option<bool>,
}

fn padding_of(d: &Val) -> [f64; 4] {
    let n = |k: &str, or: f64| d.num(k).map_or(or, f64::from);
    let left = n("stackHorizontalPadding", 0.0);
    let top = n("stackVerticalPadding", 0.0);
    [
        top,
        n("stackPaddingRight", left),
        n("stackPaddingBottom", top),
        left,
    ]
}

fn set_stack(tab: &mut Tab, ids: &[Guid], label: &str, key: Option<&str>, s: Stack) {
    let ops: Vec<EditOp> = ids
        .iter()
        .filter_map(|&id| {
            let d = &tab.doc.node(id)?.data;
            let pad = padding_of(d);
            Some(EditOp::SetAutoLayout {
                target: id,
                mode: s.mode.clone().unwrap_or_else(|| stack_mode(d)),
                spacing: s.spacing,
                counter_spacing: None,
                padding: s
                    .padding
                    .map(|p| [0, 1, 2, 3].map(|i| p[i].unwrap_or(pad[i]))),
                primary_sizing: s.primary_sizing.map(str::to_string),
                counter_sizing: s.counter_sizing.map(str::to_string),
                primary_align: s.primary_align.map(str::to_string),
                counter_align: s.counter_align.map(str::to_string),
                wrap: s.wrap,
            })
        })
        .collect();
    if !ops.is_empty() {
        let _ = apply_tab_ops(tab, &ops, Some(label), key);
    }
}

fn auto_layout(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let mode = same(tab, ids, stack_mode);
    let on = mode.as_deref().is_some_and(|m| m != "NONE");
    let add = if on { None } else { Some("Add auto layout") };
    if section_title(ui, "Auto layout", add) {
        // Figma's default: vertical, 10 gap, 10 padding, hugging both ways.
        set_stack(
            tab,
            ids,
            "Add auto layout",
            None,
            Stack {
                mode: Some("VERTICAL".into()),
                spacing: Some(10.0),
                padding: Some([Some(10.0); 4]),
                primary_sizing: Some("HUG"),
                counter_sizing: Some("HUG"),
                ..Stack::default()
            },
        );
        return;
    }
    if !on {
        ui.add_space(6.0);
        return;
    }
    section_body(ui, |ui| {
        ui.horizontal(|ui| {
            for (text, m, name) in [
                ("↓", "VERTICAL", "Vertical layout"),
                ("→", "HORIZONTAL", "Horizontal layout"),
                ("⊞", "GRID", "Grid layout"),
            ] {
                if toggle(ui, text, name, mode.as_deref() == Some(m)).clicked() {
                    set_stack(
                        tab,
                        ids,
                        "Auto layout direction",
                        None,
                        Stack {
                            mode: Some(m.into()),
                            ..Stack::default()
                        },
                    );
                }
            }
            let wrap = same(tab, ids, |d| d.text("stackWrap") == Some("WRAP")).unwrap_or(false);
            if toggle(ui, "↩", "Wrap", wrap).clicked() {
                set_stack(
                    tab,
                    ids,
                    "Wrap",
                    None,
                    Stack {
                        wrap: Some(!wrap),
                        ..Stack::default()
                    },
                );
            }
            if button(ui, "−", "Remove auto layout").clicked() {
                set_stack(
                    tab,
                    ids,
                    "Remove auto layout",
                    None,
                    Stack {
                        mode: Some("NONE".into()),
                        ..Stack::default()
                    },
                );
            }
        });
        let gap = nums(tab, ids, |d| d.num("stackSpacing").map_or(0.0, f64::from));
        ui.horizontal(|ui| {
            let icon = if mode.as_deref() == Some("VERTICAL") {
                "↕"
            } else {
                "↔"
            };
            if let Some(v) = num_field(ui, icon, "Gap", gap, 1.0, 44.0) {
                set_stack(
                    tab,
                    ids,
                    "Auto layout gap",
                    Some("design:gap"),
                    Stack {
                        spacing: Some(v.max(0.0)),
                        ..Stack::default()
                    },
                );
            }
        });
        let pads: Vec<[f64; 4]> = ids
            .iter()
            .filter_map(|id| tab.doc.node(*id).map(|n| padding_of(&n.data)))
            .collect();
        ui.horizontal(|ui| {
            for (i, (label, name)) in [
                ("T", "Top padding"),
                ("R", "Right padding"),
                ("B", "Bottom padding"),
                ("L", "Left padding"),
            ]
            .into_iter()
            .enumerate()
            {
                let v = shared(&pads.iter().map(|p| p[i]).collect::<Vec<_>>());
                if let Some(nv) = num_field(ui, label, name, v, 1.0, 28.0) {
                    let mut p = [None; 4];
                    p[i] = Some(nv.max(0.0));
                    set_stack(
                        tab,
                        ids,
                        "Auto layout padding",
                        Some("design:padding"),
                        Stack {
                            padding: Some(p),
                            ..Stack::default()
                        },
                    );
                }
            }
        });
        // Alignment and sizing are tagged by screen axis; the file stores them per layout axis.
        let vertical = mode.as_deref() == Some("VERTICAL");
        let aligns: [(&str, &str); 4] = [
            ("MIN", "Start"),
            ("CENTER", "Center"),
            ("MAX", "End"),
            ("SPACE_BETWEEN", "Space between"),
        ];
        let counter: [(&str, &str); 3] = [("MIN", "Start"), ("CENTER", "Center"), ("MAX", "End")];
        ui.horizontal(|ui| {
            let pa = same(tab, ids, |d| {
                d.text("stackPrimaryAlignItems")
                    .unwrap_or("MIN")
                    .to_string()
            });
            tag(ui, if vertical { "↕" } else { "↔" });
            if let Some(v) = combo(ui, "Main axis alignment", pa.as_deref(), &aligns, 92.0) {
                set_stack(
                    tab,
                    ids,
                    "Auto layout alignment",
                    None,
                    Stack {
                        primary_align: Some(v),
                        ..Stack::default()
                    },
                );
            }
            let ca = same(tab, ids, |d| {
                d.text("stackCounterAlignItems")
                    .unwrap_or("MIN")
                    .to_string()
            });
            tag(ui, if vertical { "↔" } else { "↕" });
            if let Some(v) = combo(ui, "Cross axis alignment", ca.as_deref(), &counter, 70.0) {
                set_stack(
                    tab,
                    ids,
                    "Auto layout alignment",
                    None,
                    Stack {
                        counter_align: Some(v),
                        ..Stack::default()
                    },
                );
            }
        });
        // Width and height: Figma names them by axis; the file by primary/counter.
        let sizes: [(&str, &str); 2] = [("FIXED", "Fixed"), ("HUG", "Hug")];
        ui.horizontal(|ui| {
            let (w_field, h_field) = if vertical {
                ("stackCounterSizing", "stackPrimarySizing")
            } else {
                ("stackPrimarySizing", "stackCounterSizing")
            };
            let w = same(tab, ids, |d| sizing(d, w_field));
            let h = same(tab, ids, |d| sizing(d, h_field));
            tag(ui, "W");
            let pw = combo(ui, "Width sizing", w, &sizes, 64.0);
            tag(ui, "H");
            let ph = combo(ui, "Height sizing", h, &sizes, 64.0);
            let (primary, counter) = if vertical { (ph, pw) } else { (pw, ph) };
            if primary.is_some() || counter.is_some() {
                set_stack(
                    tab,
                    ids,
                    "Auto layout sizing",
                    None,
                    Stack {
                        primary_sizing: primary,
                        counter_sizing: counter,
                        ..Stack::default()
                    },
                );
            }
        });
    });
}

/// How each layer sits in its parent: inside auto layout (fill, absolute), else constraints.
fn child(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let parent_stack = |id: &Guid| {
        tab.doc
            .node(*id)
            .and_then(|n| n.parent)
            .and_then(|p| tab.doc.node(p))
            .map(|p| (p.kind.clone(), stack_mode(&p.data)))
    };
    let parents: Vec<_> = ids.iter().map(parent_stack).collect();
    if parents.iter().any(Option::is_none) {
        return;
    }
    let in_stack = parents.iter().flatten().all(|(_, m)| m != "NONE");
    let in_frame = parents.iter().flatten().all(|(k, m)| {
        m == "NONE" && matches!(k.as_str(), "FRAME" | "SYMBOL" | "INSTANCE" | "SECTION")
    });
    if in_stack {
        section(ui, "In auto layout", |ui| {
            ui.horizontal(|ui| {
                let fill = same(tab, ids, |d| {
                    d.num("stackChildPrimaryGrow").unwrap_or(0.0) > 0.0
                })
                .unwrap_or(false);
                if toggle(ui, "Fill", "Fill container", fill).clicked() {
                    each(tab, ids, "Fill container", None, |id| {
                        EditOp::SetAutoLayoutChild {
                            target: id,
                            primary_fill: Some(!fill),
                            counter_stretch: None,
                            positioning: None,
                            min_size: None,
                            max_size: None,
                        }
                    });
                }
                let stretch = same(tab, ids, |d| {
                    d.text("stackChildAlignSelf") == Some("STRETCH")
                })
                .unwrap_or(false);
                if toggle(ui, "Stretch", "Stretch across", stretch).clicked() {
                    each(tab, ids, "Stretch across", None, |id| {
                        EditOp::SetAutoLayoutChild {
                            target: id,
                            primary_fill: None,
                            counter_stretch: Some(!stretch),
                            positioning: None,
                            min_size: None,
                            max_size: None,
                        }
                    });
                }
                let absolute = same(tab, ids, |d| d.text("stackPositioning") == Some("ABSOLUTE"))
                    .unwrap_or(false);
                if toggle(ui, "Absolute", "Absolute position", absolute).clicked() {
                    each(tab, ids, "Absolute position", None, |id| {
                        EditOp::SetAutoLayoutChild {
                            target: id,
                            primary_fill: None,
                            counter_stretch: None,
                            positioning: Some(if absolute { "AUTO" } else { "ABSOLUTE" }.into()),
                            min_size: None,
                            max_size: None,
                        }
                    });
                }
            });
        });
    } else if in_frame {
        section(ui, "Constraints", |ui| {
            ui.horizontal(|ui| {
                let h = same(tab, ids, |d| {
                    d.text("horizontalConstraint").unwrap_or("MIN").to_string()
                });
                let v = same(tab, ids, |d| {
                    d.text("verticalConstraint").unwrap_or("MIN").to_string()
                });
                let horizontal = [
                    ("MIN", "Left"),
                    ("MAX", "Right"),
                    ("STRETCH", "Left & right"),
                    ("CENTER", "Center"),
                    ("SCALE", "Scale"),
                ];
                let vertical = [
                    ("MIN", "Top"),
                    ("MAX", "Bottom"),
                    ("STRETCH", "Top & bottom"),
                    ("CENTER", "Center"),
                    ("SCALE", "Scale"),
                ];
                tag(ui, "↔");
                let nh = combo(ui, "Horizontal constraint", h.as_deref(), &horizontal, 96.0);
                tag(ui, "↕");
                let nv = combo(ui, "Vertical constraint", v.as_deref(), &vertical, 96.0);
                if nh.is_some() || nv.is_some() {
                    each(tab, ids, "Set constraints", None, |id| {
                        EditOp::SetConstraints {
                            target: id,
                            horizontal: nh.map(str::to_string),
                            vertical: nv.map(str::to_string),
                        }
                    });
                }
            });
        });
    }
}

fn type_op(target: Guid) -> EditOp {
    EditOp::SetTypography {
        target,
        font_family: None,
        font_style: None,
        font_size: None,
        align_horizontal: None,
        auto_resize: None,
        extra: TypographyExtra::default(),
    }
}

/// `type_op` with one change applied by `f`.
fn type_with(target: Guid, f: impl FnOnce(&mut EditOp)) -> EditOp {
    let mut op = type_op(target);
    f(&mut op);
    op
}

/// `each` for the Text section: with part of one layer's text selected on the canvas, the
/// typography a run can hold (font, size, line height, letter spacing, decoration, case)
/// goes to that range only; alignment and resizing stay the layer's.
fn each_text(
    tab: &mut Tab,
    ids: &[Guid],
    label: &str,
    key: Option<&str>,
    op: impl Fn(Guid) -> EditOp,
) {
    if let Some((target, start, end)) = crate::text_edit::utf16_range(tab)
        && ids == [target]
        && let Some(range) = to_range(&tab.doc, op(target), start, end)
    {
        let _ = apply_tab_ops(tab, &[range], Some(label), key);
        return;
    }
    each(tab, ids, label, key, op);
}

/// A `SetTypography` as a `StyleTextRange` over `start..end` (UTF-16), when every field it
/// sets is one a text run can hold.
fn to_range(doc: &figstudio_fig::Document, op: EditOp, start: usize, end: usize) -> Option<EditOp> {
    let EditOp::SetTypography {
        target,
        font_family,
        font_style,
        font_size,
        align_horizontal: None,
        auto_resize: None,
        extra,
    } = op
    else {
        return None;
    };
    let TypographyExtra {
        line_height,
        letter_spacing,
        paragraph_spacing: None,
        paragraph_indent: None,
        align_vertical: None,
        decoration,
        case,
        truncate: None,
        max_lines: None,
        ..
    } = extra
    else {
        return None;
    };
    let size = doc
        .node(target)
        .and_then(|n| n.data.num("fontSize"))
        .map_or(12.0, f64::from);
    let px = |l: TextLength| match l {
        TextLength::Px(v) => Some(v),
        TextLength::Percent(p) => Some(p / 100.0 * size),
        TextLength::Auto => None,
    };
    Some(EditOp::StyleTextRange {
        target,
        utf16_start: start,
        utf16_end: end,
        style: figstudio_fig::edit::text_runs::TextStyleRunPatch {
            font_family,
            font_style,
            font_size,
            line_height_px: line_height.and_then(px),
            letter_spacing_px: letter_spacing.and_then(px),
            text_decoration: decoration,
            text_case: case,
            ..Default::default()
        },
    })
}

fn text(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Text", |ui| {
        let family = same(tab, ids, |d| {
            d.path(&["fontName", "family"])
                .and_then(Val::as_str)
                .unwrap_or("")
                .to_string()
        });
        let style = same(tab, ids, |d| {
            d.path(&["fontName", "style"])
                .and_then(Val::as_str)
                .unwrap_or("")
                .to_string()
        });
        for (label, value, is_family) in [("Font", family, true), ("Font style", style, false)] {
            let id = egui::Id::new(("design-font", label));
            let shown = value.clone().unwrap_or_default();
            let mut t: String = ui.data(|d| d.get_temp(id)).unwrap_or(shown.clone());
            let r = ui.add(
                egui::TextEdit::singleline(&mut t)
                    .hint_text(if value.is_none() { "Mixed" } else { "" })
                    .desired_width(f32::INFINITY),
            );
            crate::a11y::name_field(&r, label);
            crate::a11y::author(&r, format!("design.{}", crate::a11y::slug(label)));
            let commit =
                r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if r.has_focus() && !commit {
                ui.data_mut(|d| d.insert_temp(id, t.clone()));
            } else {
                ui.data_mut(|d| d.remove::<String>(id));
            }
            if commit && !t.trim().is_empty() && t.trim() != shown {
                let v = t.trim().to_string();
                each_text(tab, ids, label, None, |id| {
                    type_with(id, |op| {
                        if let EditOp::SetTypography {
                            font_family,
                            font_style,
                            ..
                        } = op
                        {
                            if is_family {
                                *font_family = Some(v.clone());
                            } else {
                                *font_style = Some(v.clone());
                            }
                        }
                    })
                });
            }
        }
        let size = nums(tab, ids, |d| d.num("fontSize").map_or(12.0, f64::from));
        // In pixels; Figma's Auto (100%) is -1, shown empty with an "Auto" hint.
        let line = nums(tab, ids, |d| {
            let size = d.num("fontSize").map_or(12.0, f64::from);
            let v = d
                .path(&["lineHeight", "value"])
                .and_then(Val::as_f32)
                .map(f64::from);
            match (d.path(&["lineHeight", "units"]).and_then(Val::as_str), v) {
                (Some("PIXELS"), Some(v)) => v,
                (Some("PERCENT"), Some(v)) if v != 100.0 => v / 100.0 * size,
                (Some("RAW"), Some(v)) if v != 1.0 => v * size,
                _ => -1.0,
            }
        });
        let spacing = nums(tab, ids, |d| {
            d.path(&["letterSpacing", "value"])
                .and_then(Val::as_f32)
                .map_or(0.0, f64::from)
        });
        ui.horizontal(|ui| {
            if let Some(v) = num_field(ui, "Aa", "Font size", size, 0.5, 36.0) {
                each_text(tab, ids, "Font size", Some("design:font-size"), |id| {
                    type_with(id, |op| {
                        if let EditOp::SetTypography { font_size, .. } = op {
                            *font_size = Some(v.clamp(1.0, 1000.0));
                        }
                    })
                });
            }
            let shown = line.filter(|v| *v >= 0.0);
            let hint = if line.is_some() { "Auto" } else { "Mixed" };
            if let Some(v) = num_field_or(ui, "↕", "Line height", shown, 0.5, 36.0, hint) {
                each_text(tab, ids, "Line height", Some("design:line-height"), |id| {
                    type_with(id, |op| {
                        if let EditOp::SetTypography { extra, .. } = op {
                            extra.line_height = Some(TextLength::Px(v.max(0.0)));
                        }
                    })
                });
            }
            if let Some(v) = num_field(ui, "|A|", "Letter spacing", spacing, 0.1, 36.0) {
                each_text(
                    tab,
                    ids,
                    "Letter spacing",
                    Some("design:letter-spacing"),
                    |id| {
                        type_with(id, |op| {
                            if let EditOp::SetTypography { extra, .. } = op {
                                extra.letter_spacing = Some(TextLength::Percent(v));
                            }
                        })
                    },
                );
            }
        });
        ui.horizontal(|ui| {
            let align = same(tab, ids, |d| {
                d.text("textAlignHorizontal").unwrap_or("LEFT").to_string()
            });
            for (text, a, name) in [
                ("⇤", "LEFT", "Align text left"),
                ("≡", "CENTER", "Align text center"),
                ("⇥", "RIGHT", "Align text right"),
                ("☰", "JUSTIFIED", "Justify text"),
            ] {
                if toggle(ui, text, name, align.as_deref() == Some(a)).clicked() {
                    each_text(tab, ids, "Text alignment", None, |id| {
                        type_with(id, |op| {
                            if let EditOp::SetTypography {
                                align_horizontal, ..
                            } = op
                            {
                                *align_horizontal = Some(a.into());
                            }
                        })
                    });
                }
            }
            let valign = same(tab, ids, |d| {
                d.text("textAlignVertical").unwrap_or("TOP").to_string()
            });
            for (text, a, name) in [
                ("⤒", "TOP", "Align text top"),
                ("⇕", "CENTER", "Align text middle"),
                ("⤓", "BOTTOM", "Align text bottom"),
            ] {
                if toggle(ui, text, name, valign.as_deref() == Some(a)).clicked() {
                    each_text(tab, ids, "Text alignment", None, |id| {
                        type_with(id, |op| {
                            if let EditOp::SetTypography { extra, .. } = op {
                                extra.align_vertical = Some(a.into());
                            }
                        })
                    });
                }
            }
        });
        ui.horizontal(|ui| {
            let resize = same(tab, ids, |d| {
                d.text("textAutoResize").unwrap_or("NONE").to_string()
            });
            let modes = [
                ("WIDTH_AND_HEIGHT", "Auto width"),
                ("HEIGHT", "Auto height"),
                ("NONE", "Fixed size"),
            ];
            if let Some(v) = combo(ui, "Text resizing", resize.as_deref(), &modes, 92.0) {
                each_text(tab, ids, "Text resizing", None, |id| {
                    type_with(id, |op| {
                        if let EditOp::SetTypography { auto_resize, .. } = op {
                            *auto_resize = Some(v.into());
                        }
                    })
                });
            }
            let deco = same(tab, ids, |d| {
                d.text("textDecoration").unwrap_or("NONE").to_string()
            });
            let decos = [
                ("NONE", "No line"),
                ("UNDERLINE", "Underline"),
                ("STRIKETHROUGH", "Strikethrough"),
            ];
            if let Some(v) = combo(ui, "Text decoration", deco.as_deref(), &decos, 92.0) {
                each_text(tab, ids, "Text decoration", None, |id| {
                    type_with(id, |op| {
                        if let EditOp::SetTypography { extra, .. } = op {
                            extra.decoration = Some(v.into());
                        }
                    })
                });
            }
        });
        let case = same(tab, ids, |d| {
            d.text("textCase").unwrap_or("ORIGINAL").to_string()
        });
        let cases = [
            ("ORIGINAL", "As typed"),
            ("UPPER", "Uppercase"),
            ("LOWER", "Lowercase"),
            ("TITLE", "Title case"),
        ];
        if let Some(v) = combo(ui, "Letter case", case.as_deref(), &cases, 92.0) {
            each_text(tab, ids, "Letter case", None, |id| {
                type_with(id, |op| {
                    if let EditOp::SetTypography { extra, .. } = op {
                        extra.case = Some(v.into());
                    }
                })
            });
        }
        // The characters, for one layer (or double-click the text on the canvas).
        if let [id] = ids {
            let id = *id;
            let chars = tab
                .doc
                .node(id)
                .and_then(|n| {
                    n.data
                        .path(&["textData", "characters"])
                        .and_then(Val::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let mut t = chars.clone();
            let r = ui.add(
                egui::TextEdit::multiline(&mut t)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            crate::a11y::name_field(&r, "Text content");
            crate::a11y::author(&r, "design.text-content");
            if r.changed() && t != chars {
                let _ = apply_tab_ops(
                    tab,
                    &[EditOp::SetText {
                        target: id,
                        characters: t,
                    }],
                    Some("Edit text"),
                    Some(&format!("design:text:{id}")),
                );
            }
        }
    });
}

fn instance(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Instance", |ui| {
        ui.horizontal(|ui| {
            if button(ui, "Reset", "Reset overrides").clicked() {
                each(tab, ids, "Reset overrides", None, |id| {
                    EditOp::ResetInstanceOverride {
                        instance: id,
                        override_path: None,
                        field: None,
                    }
                });
            }
            if button(ui, "Detach", "Detach instance").clicked() {
                let _ = super::commands::run(tab, super::commands::EditCmd::DetachInstance);
            }
        });
    });
}

/// Export settings and the export button (the app runs the export).
pub(super) fn export(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let lists: Vec<Vec<Val>> = ids
        .iter()
        .filter_map(|id| tab.doc.node(*id))
        .map(|n| {
            n.data
                .get("exportSettings")
                .map(|v| v.items().iter().map(|e| e.clone().into_owned()).collect())
                .unwrap_or_default()
        })
        .collect();
    let list = lists
        .first()
        .filter(|f| lists.iter().all(|l| l == *f))
        .cloned();
    let specs: Vec<ExportSpec> = list
        .iter()
        .flatten()
        .map(|e| ExportSpec {
            format: match e.text("imageType") {
                Some("JPEG") => "jpg",
                Some("SVG") => "svg",
                Some("PDF") => "pdf",
                _ => "png",
            }
            .into(),
            scale: (e.path(&["constraint", "type"]).and_then(Val::as_str) == Some("CONTENT_SCALE"))
                .then(|| {
                    e.path(&["constraint", "value"])
                        .and_then(Val::as_f32)
                        .map_or(1.0, f64::from)
                }),
            width: None,
            height: None,
            suffix: e
                .text("suffix")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        })
        .collect();
    let write = |tab: &mut Tab, settings: Vec<ExportSpec>| {
        each(tab, ids, "Export settings", None, |id| {
            EditOp::SetExportSettings {
                target: id,
                settings: settings.clone(),
            }
        });
    };
    if section_title(ui, "Export", Some("Add export setting")) {
        let mut next = specs.clone();
        next.push(ExportSpec {
            format: "png".into(),
            scale: Some(if specs.is_empty() { 1.0 } else { 2.0 }),
            width: None,
            height: None,
            suffix: None,
        });
        write(tab, next);
        return;
    }
    if specs.is_empty() {
        ui.add_space(6.0);
        return;
    }
    section_body(ui, |ui| {
        for (i, spec) in specs.iter().enumerate() {
            ui.horizontal(|ui| {
                let scales: [(&str, &str); 5] = [
                    ("0.5", "0.5x"),
                    ("1", "1x"),
                    ("2", "2x"),
                    ("3", "3x"),
                    ("4", "4x"),
                ];
                let cur = spec.scale.map(super::widgets::fmt);
                let n = format!("Export {} scale", i + 1);
                if let Some(v) = combo(ui, &n, cur.as_deref(), &scales, 52.0) {
                    let mut next = specs.clone();
                    next[i].scale = v.parse().ok();
                    write(tab, next);
                }
                let formats: [(&str, &str); 4] = [
                    ("png", "PNG"),
                    ("jpg", "JPG"),
                    ("svg", "SVG"),
                    ("pdf", "PDF"),
                ];
                let n = format!("Export {} format", i + 1);
                if let Some(v) = combo(ui, &n, Some(spec.format.as_str()), &formats, 56.0) {
                    let mut next = specs.clone();
                    next[i].format = v.into();
                    write(tab, next);
                }
                if button(ui, "−", &format!("Remove export {}", i + 1)).clicked() {
                    let mut next = specs.clone();
                    next.remove(i);
                    write(tab, next);
                }
            });
        }
        // The app exports PNG and SVG of the selection; the first matching setting decides.
        let action = specs.iter().find_map(|s| match s.format.as_str() {
            "png" => Some(crate::menu::Action::ExportSelection(s.scale.unwrap_or(1.0))),
            "svg" => Some(crate::menu::Action::ExportSelectionSvg),
            _ => None,
        });
        let label = if ids.len() == 1 {
            "Export layer".to_string()
        } else {
            format!("Export {} layers", ids.len())
        };
        let r = ui.add_enabled(action.is_some(), egui::Button::new(label.as_str()));
        crate::a11y::author(&r, "design.export");
        if r.clicked() {
            tab.pending_action = action;
        }
    });
}
