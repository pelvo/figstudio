//! The Design panel's component sections: an instance's main component (swap), variant and
//! property values; a component's property definitions and variants; and, for a layer inside
//! a component, which properties drive it.

use egui::Ui;
use figstudio_fig::edit::component_props::{
    PropView, PropViewValue, component_props, instance_props,
};
use figstudio_fig::edit::instances::ComponentPropVal;
use figstudio_fig::edit::{ComponentPropKind, EditOp};
use figstudio_fig::{Document, Guid, Val};

use crate::panels::section;
use crate::tab::Tab;

use super::apply_tab_ops;
use super::widgets::{button, text_field};

/// Every local component with a label: `Set / Variant` for a variant, else its name; sorted.
pub(crate) fn local_components(doc: &Document) -> Vec<(Guid, String)> {
    let mut out: Vec<(Guid, String)> = doc
        .nodes
        .iter()
        .filter(|n| n.kind == "SYMBOL")
        .map(|n| {
            let set = n
                .parent
                .and_then(|p| doc.node(p))
                .filter(|p| figstudio_fig::edit::component_props::is_variant_set(p));
            let label = match set {
                Some(set) => format!("{} / {}", set.name, n.name),
                None => n.name.clone(),
            };
            (n.id, label)
        })
        .collect();
    out.sort_by_key(|(_, label)| label.to_lowercase());
    out
}

/// A drop-down of `options` (value, label) showing `current`; returns the value picked.
pub(super) fn pick<T: Clone + PartialEq>(
    ui: &mut Ui,
    name: &str,
    current: Option<&T>,
    options: &[(T, String)],
) -> Option<T> {
    let shown = current
        .and_then(|c| options.iter().find(|(v, _)| v == c))
        .map_or("None", |(_, l)| l.as_str());
    let mut picked = None;
    let r = egui::ComboBox::from_id_salt(("design-pick", name))
        .selected_text(shown)
        .width(ui.available_width().min(170.0))
        .show_ui(ui, |ui| {
            for (v, l) in options {
                if ui.selectable_label(Some(v) == current, l).clicked() {
                    picked = Some(v.clone());
                }
            }
        });
    crate::a11y::name_field(&r.response, name);
    crate::a11y::author(&r.response, format!("design.{}", crate::a11y::slug(name)));
    picked.filter(|p| Some(p) != current)
}

fn prop_key(p: &PropView) -> String {
    p.id.map_or_else(|| p.name.clone(), |g| g.to_string())
}

fn run(tab: &mut Tab, op: EditOp, label: &str) {
    let _ = apply_tab_ops(tab, &[op], Some(label), None);
}

/// The instance section: main component (swap), variants, property values, reset, detach.
pub(super) fn instance(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Instance", |ui| {
        if let [inst] = ids {
            let inst = *inst;
            let main = tab
                .doc
                .node(inst)
                .and_then(|n| figstudio_fig::resolve::symbol_id(&n.data));
            let components = crate::assets::index(ui.ctx(), tab).components.clone();
            ui.horizontal(|ui| {
                if let Some(to) = pick(ui, "Main component", main.as_ref(), &components) {
                    run(
                        tab,
                        EditOp::SwapInstance {
                            instance: inst,
                            symbol: to,
                        },
                        "Swap instance",
                    );
                }
                if let Some(main) = main
                    && button(ui, "↗", "Go to main component").clicked()
                {
                    tab.show_node(main);
                }
            });
            for p in instance_props(&tab.doc, inst) {
                value_row(ui, tab, &p, &components, |val| match val {
                    Change::Variant(v) => EditOp::SwitchVariant {
                        instance: inst,
                        variants: vec![(p.name.clone(), v)],
                    },
                    Change::Value(val) => EditOp::SetComponentProp {
                        instance: inst,
                        prop: prop_key(&p),
                        val,
                    },
                });
            }
        }
        ui.horizontal(|ui| {
            if button(ui, "Reset", "Reset overrides").clicked() {
                let ops: Vec<EditOp> = ids
                    .iter()
                    .map(|&id| EditOp::ResetInstanceOverride {
                        instance: id,
                        override_path: None,
                        field: None,
                    })
                    .collect();
                let _ = apply_tab_ops(tab, &ops, Some("Reset overrides"), None);
            }
            if button(ui, "Detach", "Detach instance").clicked() {
                let _ = super::commands::run(tab, super::commands::EditCmd::DetachInstance);
            }
        });
    });
}

/// What a property row changed.
enum Change {
    Variant(String),
    Value(ComponentPropVal),
}

/// One property: a variant drop-down, a checkbox, a text field or a component drop-down.
fn value_row(
    ui: &mut Ui,
    tab: &mut Tab,
    p: &PropView,
    components: &[(Guid, String)],
    op: impl Fn(Change) -> EditOp,
) {
    let label = format!("{} value", p.name);
    let change = ui
        .horizontal(|ui| {
            ui.label(egui::RichText::new(&p.name).color(crate::theme::of(ui).text_dim));
            match &p.value {
                PropViewValue::Variant { value, options } => {
                    let opts: Vec<(String, String)> =
                        options.iter().map(|o| (o.clone(), o.clone())).collect();
                    pick(ui, &label, Some(value), &opts).map(Change::Variant)
                }
                PropViewValue::Bool(b) => {
                    let mut v = *b;
                    let r = ui.checkbox(&mut v, "");
                    crate::a11y::name_field(&r, &label);
                    crate::a11y::author(&r, format!("design.{}", crate::a11y::slug(&label)));
                    r.changed()
                        .then_some(Change::Value(ComponentPropVal::Bool(v)))
                }
                PropViewValue::Text(t) => text_field(ui, &label, t, ui.available_width())
                    .map(|t| Change::Value(ComponentPropVal::Text(t))),
                PropViewValue::InstanceSwap(g) => pick(ui, &label, g.as_ref(), components)
                    .map(|g| Change::Value(ComponentPropVal::InstanceSwap(g))),
            }
        })
        .inner;
    if let Some(c) = change {
        run(tab, op(c), "Set property");
    }
}

/// The component (or variant set) section: its properties (rename, default, remove, add)
/// and its variants (add one).
pub(super) fn component(ui: &mut Ui, tab: &mut Tab, id: Guid) {
    let doc = &tab.doc;
    let Some(node) = doc.node(id) else { return };
    let set = figstudio_fig::edit::component_props::is_variant_set(node);
    let in_set = node
        .parent
        .and_then(|p| doc.node(p))
        .is_some_and(figstudio_fig::edit::component_props::is_variant_set);
    let first_variant = if set {
        doc.children(id)
            .iter()
            .copied()
            .find(|c| doc.node(*c).is_some_and(|n| n.kind == "SYMBOL"))
    } else {
        in_set.then_some(id)
    };
    section(ui, "Component properties", |ui| {
        let components = crate::assets::index(ui.ctx(), tab).components.clone();
        for p in component_props(&tab.doc, id) {
            if let PropViewValue::Variant { options, .. } = &p.value {
                ui.label(format!("{}: {}", p.name, options.join(", ")));
                continue;
            }
            ui.horizontal(|ui| {
                if let Some(name) = text_field(ui, &format!("{} name", p.name), &p.name, 80.0) {
                    run(
                        tab,
                        EditOp::EditComponentProp {
                            component: id,
                            prop: prop_key(&p),
                            name: Some(name),
                            default: None,
                        },
                        "Rename property",
                    );
                }
                if button(ui, "−", &format!("Remove {}", p.name)).clicked() {
                    run(
                        tab,
                        EditOp::RemoveComponentProp {
                            component: id,
                            prop: prop_key(&p),
                        },
                        "Remove property",
                    );
                }
            });
            value_row(ui, tab, &p, &components, |c| EditOp::EditComponentProp {
                component: id,
                prop: prop_key(&p),
                name: None,
                default: match c {
                    Change::Value(v) => Some(v),
                    Change::Variant(_) => None,
                },
            });
        }
        ui.horizontal(|ui| {
            for (text, kind, default) in [
                (
                    "+ Boolean",
                    ComponentPropKind::Bool,
                    ComponentPropVal::Bool(true),
                ),
                (
                    "+ Text",
                    ComponentPropKind::Text,
                    ComponentPropVal::Text("Text".into()),
                ),
            ] {
                if button(ui, text, &format!("Add {} property", &text[2..])).clicked() {
                    let name =
                        format!("{} {}", &text[2..], component_props(&tab.doc, id).len() + 1);
                    run(
                        tab,
                        EditOp::AddComponentProp {
                            component: id,
                            name,
                            kind,
                            default,
                        },
                        "Add property",
                    );
                }
            }
            if let Some(&(first, _)) = components.first()
                && button(ui, "+ Swap", "Add Instance swap property").clicked()
            {
                run(
                    tab,
                    EditOp::AddComponentProp {
                        component: id,
                        name: format!("Instance {}", component_props(&tab.doc, id).len() + 1),
                        kind: ComponentPropKind::InstanceSwap,
                        default: ComponentPropVal::InstanceSwap(first),
                    },
                    "Add property",
                );
            }
        });
        if let Some(from) = first_variant
            && button(ui, "+ Variant", "Add variant").clicked()
        {
            run(tab, EditOp::AddVariant { from }, "Add variant");
        }
    });
}

/// Several components: combine them into a variant set.
pub(super) fn combine(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    section(ui, "Components", |ui| {
        if button(ui, "Combine as variants", "Combine as variants").clicked()
            && let Ok(created) = apply_tab_ops(
                tab,
                &[EditOp::CombineAsVariants {
                    components: ids.to_vec(),
                    name: None,
                }],
                Some("Combine as variants"),
                None,
            )
            && let Some(&set) = created.first()
        {
            tab.select(Some(set));
        }
    });
}

/// The component a layer sits in (not crossing an instance), and that layer.
fn host_component(doc: &Document, id: Guid) -> Option<Guid> {
    let mut cur = doc.node(id)?.parent;
    for _ in 0..4096 {
        let n = doc.node(cur?)?;
        match n.kind.as_str() {
            "SYMBOL" => return Some(n.id),
            "INSTANCE" | "CANVAS" => return None,
            _ => cur = n.parent,
        }
    }
    None
}

/// The property a layer's `field` is bound to.
fn bound(doc: &Document, id: Guid, field: &str) -> Option<Guid> {
    doc.node(id)?
        .data
        .get("componentPropRefs")?
        .items()
        .iter()
        .find(|r| {
            r.text("componentPropNodeField") == Some(field) && r.flag("isDeleted") != Some(true)
        })
        .and_then(|r| r.get("defID"))
        .and_then(Val::as_guid)
}

/// A layer inside a component: which properties drive its visibility, its text (text
/// layers) and its main component (instances).
pub(super) fn bindings(ui: &mut Ui, tab: &mut Tab, id: Guid) {
    let doc = &tab.doc;
    let Some(host) = host_component(doc, id) else {
        return;
    };
    let props = component_props(doc, host);
    let kind = doc.node(id).map(|n| n.kind.clone()).unwrap_or_default();
    let rows: Vec<(&str, &str, ComponentPropKind)> = [
        ("Visible", "VISIBLE", ComponentPropKind::Bool),
        ("Text", "TEXT_DATA", ComponentPropKind::Text),
        (
            "Swap",
            "OVERRIDDEN_SYMBOL_ID",
            ComponentPropKind::InstanceSwap,
        ),
    ]
    .into_iter()
    .filter(|(_, _, k)| match k {
        ComponentPropKind::Text => kind == "TEXT",
        ComponentPropKind::InstanceSwap => kind == "INSTANCE",
        ComponentPropKind::Bool => true,
    })
    .filter(|(_, _, k)| props.iter().any(|p| matches_kind(&p.value, *k)))
    .collect();
    if rows.is_empty() {
        return;
    }
    section(ui, "Component property", |ui| {
        for (label, field, k) in rows {
            let options: Vec<(Option<Guid>, String)> = std::iter::once((None, "None".to_string()))
                .chain(
                    props
                        .iter()
                        .filter(|p| matches_kind(&p.value, k))
                        .filter_map(|p| p.id.map(|g| (Some(g), p.name.clone()))),
                )
                .collect();
            let current = bound(&tab.doc, id, field);
            let picked = ui
                .horizontal(|ui| {
                    ui.label(egui::RichText::new(label).color(crate::theme::of(ui).text_dim));
                    pick(ui, &format!("{label} property"), Some(&current), &options)
                })
                .inner;
            match picked {
                Some(Some(prop)) => run(
                    tab,
                    EditOp::BindComponentProp {
                        node: id,
                        prop: prop.to_string(),
                    },
                    "Bind property",
                ),
                Some(None) => run(
                    tab,
                    EditOp::UnbindComponentProp { node: id, kind: k },
                    "Unbind property",
                ),
                None => {}
            }
        }
    });
}

fn matches_kind(v: &PropViewValue, k: ComponentPropKind) -> bool {
    matches!(
        (v, k),
        (PropViewValue::Bool(_), ComponentPropKind::Bool)
            | (PropViewValue::Text(_), ComponentPropKind::Text)
            | (
                PropViewValue::InstanceSwap(_),
                ComponentPropKind::InstanceSwap
            )
    )
}
