//! The Design panel's Boolean & mask buttons and Prototype section (interactions, flow start,
//! scrolling), and the selection's connections drawn on the canvas.

use egui::{Color32, Pos2, Stroke, Ui, pos2, vec2};
use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{Action, EditOp, MaskKind, ReactionSpec, Trigger, reactions_of};
use figstudio_fig::{Document, Guid};

use crate::panels::section;
use crate::tab::Tab;

use super::apply_tab_ops;
use super::commands::{EditCmd, run};
use super::design_component::pick;
use super::widgets::{button, num_field, text_field};

fn run_op(tab: &mut Tab, op: EditOp, label: &str) {
    let _ = apply_tab_ops(tab, &[op], Some(label), None);
}

/// Union, Subtract, Intersect, Exclude and Use as mask for the selection; for a mask, its kind.
pub(super) fn boolean_mask(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let mask = match ids {
        [id] => tab
            .doc
            .node(*id)
            .filter(|n| n.data.flag("mask") == Some(true))
            .map(
                |n| match (n.data.text("maskType"), n.data.flag("maskIsOutline")) {
                    (_, Some(true)) | (Some("OUTLINE"), _) => "outline",
                    (Some("LUMINANCE"), _) => "luminance",
                    _ => "alpha",
                },
            ),
        _ => None,
    };
    if ids.len() < 2 && mask.is_none() {
        return;
    }
    section(ui, "Boolean & mask", |ui| {
        if ids.len() >= 2 {
            ui.horizontal(|ui| {
                for (text, op) in [
                    ("Union", BooleanOp::Union),
                    ("Subtract", BooleanOp::Subtract),
                    ("Intersect", BooleanOp::Intersect),
                    ("Exclude", BooleanOp::Exclude),
                ] {
                    let cmd = EditCmd::Boolean(op);
                    let r = ui
                        .add_enabled_ui(cmd.enabled(tab), |ui| button(ui, text, cmd.label()))
                        .inner;
                    if r.clicked() {
                        let _ = run(tab, cmd);
                    }
                }
            });
            // Figma masks with the bottom layer: it is the one the command turns into a mask.
            if button(ui, "Use as mask", "Mask with the bottom layer").clicked() {
                let bottom = ids.iter().copied().min_by_key(|id| {
                    tab.doc
                        .node(*id)
                        .and_then(|n| n.parent)
                        .map(|p| tab.doc.children(p).iter().position(|c| c == id))
                });
                if let Some(b) = bottom {
                    run_op(
                        tab,
                        EditOp::SetMask {
                            target: b,
                            mask: Some(MaskKind::Alpha),
                        },
                        "Use as mask",
                    );
                }
            }
        }
        if let (Some(kind), [id]) = (mask, ids) {
            let id = *id;
            ui.horizontal(|ui| {
                let kinds = [
                    ("alpha".to_string(), "Alpha".to_string()),
                    ("outline".to_string(), "Outline".to_string()),
                    ("luminance".to_string(), "Luminance".to_string()),
                ];
                if let Some(k) = pick(ui, "Mask type", Some(&kind.to_string()), &kinds) {
                    let mask = match k.as_str() {
                        "outline" => MaskKind::Outline,
                        "luminance" => MaskKind::Luminance,
                        _ => MaskKind::Alpha,
                    };
                    run_op(
                        tab,
                        EditOp::SetMask {
                            target: id,
                            mask: Some(mask),
                        },
                        "Mask type",
                    );
                }
                if button(ui, "Stop masking", "Stop masking").clicked() {
                    run_op(
                        tab,
                        EditOp::SetMask {
                            target: id,
                            mask: None,
                        },
                        "Stop masking",
                    );
                }
            });
        }
    });
}

fn trigger_key(t: &Trigger) -> String {
    match t {
        Trigger::OnClick => "on_click",
        Trigger::OnHover => "on_hover",
        Trigger::OnPress => "on_press",
        Trigger::OnDrag => "on_drag",
        Trigger::MouseEnter => "mouse_enter",
        Trigger::MouseLeave => "mouse_leave",
        Trigger::MouseDown => "mouse_down",
        Trigger::MouseUp => "mouse_up",
        Trigger::AfterTimeout { .. } => "after_delay",
    }
    .into()
}

fn trigger_of(key: &str) -> Trigger {
    match key {
        "on_hover" => Trigger::OnHover,
        "on_press" => Trigger::OnPress,
        "on_drag" => Trigger::OnDrag,
        "mouse_enter" => Trigger::MouseEnter,
        "mouse_leave" => Trigger::MouseLeave,
        "mouse_down" => Trigger::MouseDown,
        "mouse_up" => Trigger::MouseUp,
        "after_delay" => Trigger::AfterTimeout { ms: 800.0 },
        _ => Trigger::OnClick,
    }
}

fn action_key(a: &Action) -> String {
    match a {
        Action::Navigate { .. } => "navigate",
        Action::Overlay { .. } => "overlay",
        Action::Swap { .. } => "swap",
        Action::ScrollTo { .. } => "scroll_to",
        Action::Back => "back",
        Action::Close => "close",
        Action::Url { .. } => "url",
    }
    .into()
}

fn destination(a: &Action) -> Option<Guid> {
    match a {
        Action::Navigate { to }
        | Action::Overlay { to }
        | Action::Swap { to }
        | Action::ScrollTo { to } => Some(*to),
        _ => None,
    }
}

/// The top-level frame holding `id` (itself when it is one).
fn top_frame(doc: &Document, id: Guid) -> Option<Guid> {
    let mut cur = id;
    for _ in 0..4096 {
        let n = doc.node(cur)?;
        let parent = n.parent?;
        if doc.node(parent)?.kind == "CANVAS" {
            return Some(cur);
        }
        cur = parent;
    }
    None
}

/// Where an action can go: the page's top-level frames (for Scroll to, the layers in this
/// layer's frame).
fn destinations(tab: &Tab, id: Guid, scroll: bool) -> Vec<(Guid, String)> {
    let doc = &tab.doc;
    if scroll {
        let mut out = Vec::new();
        let mut stack: Vec<Guid> = top_frame(doc, id)
            .map(|f| doc.children(f).to_vec())
            .unwrap_or_default();
        while let Some(c) = stack.pop() {
            if out.len() >= 300 {
                break;
            }
            if let Some(n) = doc.node(c) {
                out.push((c, n.name.clone()));
                stack.extend(doc.children(c).iter().copied());
            }
        }
        return out;
    }
    doc.children(tab.page)
        .iter()
        .filter_map(|c| doc.node(*c))
        .filter(|n| matches!(n.kind.as_str(), "FRAME" | "SYMBOL" | "INSTANCE" | "SECTION"))
        .map(|n| (n.id, n.name.clone()))
        .collect()
}

/// The Prototype section: interactions, and for frames the flow start and scrolling.
pub(super) fn prototype(ui: &mut Ui, tab: &mut Tab, ids: &[Guid]) {
    let [id] = ids else { return };
    let id = *id;
    let doc = &tab.doc;
    let Some(node) = doc.node(id) else { return };
    let top_level = node
        .parent
        .and_then(|p| doc.node(p))
        .is_some_and(|p| p.kind == "CANVAS");
    let is_frame = matches!(node.kind.as_str(), "FRAME" | "SYMBOL" | "INSTANCE");
    let in_frame = node
        .parent
        .and_then(|p| doc.node(p))
        .is_some_and(|p| matches!(p.kind.as_str(), "FRAME" | "SYMBOL" | "INSTANCE"));
    let reactions = reactions_of(doc, id);
    let stored = node
        .data
        .get("prototypeInteractions")
        .map_or(0, |v| v.items().len());
    let flow = node
        .data
        .path(&["prototypeStartingPoint", "name"])
        .and_then(figstudio_fig::Val::as_str)
        .map(str::to_owned);
    let overflow = node
        .data
        .text("scrollDirection")
        .unwrap_or("NONE")
        .to_ascii_lowercase();
    let behavior = match node.data.text("scrollBehavior") {
        Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME") => "fixed",
        Some("STICKY_SCROLLS") => "sticky",
        _ => "scrolls",
    };
    let add = reactions.is_some().then_some("Add interaction");
    if crate::panels::section_title(ui, "Prototype", add) {
        let to = destinations(tab, id, false)
            .into_iter()
            .map(|(g, _)| g)
            .find(|g| Some(*g) != top_frame(&tab.doc, id));
        let mut list = reactions.clone().unwrap_or_default();
        list.push(ReactionSpec {
            trigger: Trigger::OnClick,
            action: to.map_or(Action::Back, |to| Action::Navigate { to }),
            transition: Some("instant".into()),
            duration: None,
            easing: None,
        });
        run_op(
            tab,
            EditOp::SetReactions {
                target: id,
                reactions: list,
            },
            "Add interaction",
        );
        return;
    }
    crate::panels::section_body(ui, |ui| {
        match &reactions {
            Some(list) => {
                for (i, r) in list.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    interaction(ui, tab, id, list, i, r);
                }
            }
            None => {
                ui.label(format!(
                    "{stored} interactions this panel cannot edit (several actions, or a kind it does not know); they are kept."
                ));
            }
        }
        if top_level {
            ui.horizontal(|ui| {
                let mut on = flow.is_some();
                let r = ui.checkbox(&mut on, "Flow starting point");
                crate::a11y::author(&r, "design.flow-start");
                if r.changed() {
                    let name = on.then(|| "Flow 1".to_string());
                    run_op(
                        tab,
                        EditOp::SetFlowStart { frame: id, name },
                        "Flow starting point",
                    );
                }
                if let Some(name) = &flow
                    && let Some(n) = text_field(ui, "Flow name", name, 100.0)
                {
                    run_op(
                        tab,
                        EditOp::SetFlowStart {
                            frame: id,
                            name: Some(n),
                        },
                        "Rename flow",
                    );
                }
            });
        }
        if is_frame {
            ui.horizontal(|ui| {
                ui.label("Overflow");
                let opts: Vec<(String, String)> = [
                    ("none", "No scrolling"),
                    ("horizontal", "Horizontal"),
                    ("vertical", "Vertical"),
                    ("both", "Both directions"),
                ]
                .iter()
                .map(|(k, l)| (k.to_string(), l.to_string()))
                .collect();
                if let Some(v) = pick(ui, "Overflow scrolling", Some(&overflow), &opts) {
                    run_op(
                        tab,
                        EditOp::SetScrollBehavior {
                            target: id,
                            overflow: Some(v),
                            behavior: None,
                        },
                        "Overflow scrolling",
                    );
                }
            });
        }
        if in_frame {
            ui.horizontal(|ui| {
                ui.label("When scrolling");
                let opts: Vec<(String, String)> = [
                    ("scrolls", "Scroll with parent"),
                    ("fixed", "Fixed"),
                    ("sticky", "Sticky"),
                ]
                .iter()
                .map(|(k, l)| (k.to_string(), l.to_string()))
                .collect();
                if let Some(v) = pick(
                    ui,
                    "Position when scrolling",
                    Some(&behavior.to_string()),
                    &opts,
                ) {
                    run_op(
                        tab,
                        EditOp::SetScrollBehavior {
                            target: id,
                            overflow: None,
                            behavior: Some(v),
                        },
                        "Scroll behavior",
                    );
                }
            });
        }
    });
}

/// One interaction: trigger, action and destination, transition and duration, remove.
fn interaction(
    ui: &mut Ui,
    tab: &mut Tab,
    id: Guid,
    list: &[ReactionSpec],
    i: usize,
    r: &ReactionSpec,
) {
    let n = i + 1;
    let write = |tab: &mut Tab, r: ReactionSpec, label: &str| {
        let mut all = list.to_vec();
        all[i] = r;
        run_op(
            tab,
            EditOp::SetReactions {
                target: id,
                reactions: all,
            },
            label,
        );
    };
    ui.horizontal(|ui| {
        let triggers: Vec<(String, String)> = [
            ("on_click", "On click"),
            ("on_hover", "While hovering"),
            ("on_press", "While pressing"),
            ("on_drag", "On drag"),
            ("mouse_enter", "Mouse enter"),
            ("mouse_leave", "Mouse leave"),
            ("mouse_down", "Mouse down"),
            ("mouse_up", "Mouse up"),
            ("after_delay", "After delay"),
        ]
        .iter()
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect();
        if let Some(k) = pick(
            ui,
            &format!("Interaction {n} trigger"),
            Some(&trigger_key(&r.trigger)),
            &triggers,
        ) {
            write(
                tab,
                ReactionSpec {
                    trigger: trigger_of(&k),
                    ..r.clone()
                },
                "Interaction trigger",
            );
        }
        if let Trigger::AfterTimeout { ms } = r.trigger
            && let Some(v) = num_field(
                ui,
                "ms",
                &format!("Interaction {n} delay"),
                Some(ms),
                10.0,
                44.0,
            )
        {
            write(
                tab,
                ReactionSpec {
                    trigger: Trigger::AfterTimeout { ms: v.max(0.0) },
                    ..r.clone()
                },
                "Interaction delay",
            );
        }
        if button(ui, "−", &format!("Remove interaction {n}")).clicked() {
            let mut all = list.to_vec();
            all.remove(i);
            run_op(
                tab,
                EditOp::SetReactions {
                    target: id,
                    reactions: all,
                },
                "Remove interaction",
            );
        }
    });
    ui.horizontal(|ui| {
        let actions: Vec<(String, String)> = [
            ("navigate", "Navigate to"),
            ("overlay", "Open overlay"),
            ("swap", "Swap with"),
            ("scroll_to", "Scroll to"),
            ("back", "Back"),
            ("close", "Close overlay"),
            ("url", "Open link"),
        ]
        .iter()
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect();
        let key = action_key(&r.action);
        if let Some(k) = pick(ui, &format!("Interaction {n} action"), Some(&key), &actions) {
            let first = destinations(tab, id, k == "scroll_to")
                .into_iter()
                .map(|(g, _)| g)
                .find(|g| Some(*g) != top_frame(&tab.doc, id));
            let to = destination(&r.action).or(first);
            let action = match (k.as_str(), to) {
                ("navigate", Some(to)) => Action::Navigate { to },
                ("overlay", Some(to)) => Action::Overlay { to },
                ("swap", Some(to)) => Action::Swap { to },
                ("scroll_to", Some(to)) => Action::ScrollTo { to },
                ("close", _) => Action::Close,
                ("url", _) => Action::Url {
                    url: "https://".into(),
                },
                _ => Action::Back,
            };
            write(
                tab,
                ReactionSpec {
                    action,
                    ..r.clone()
                },
                "Interaction action",
            );
        }
        match &r.action {
            Action::Url { url } => {
                if let Some(u) = text_field(
                    ui,
                    &format!("Interaction {n} link"),
                    url,
                    ui.available_width(),
                ) {
                    write(
                        tab,
                        ReactionSpec {
                            action: Action::Url { url: u },
                            ..r.clone()
                        },
                        "Interaction link",
                    );
                }
            }
            a => {
                if let Some(to) = destination(a) {
                    let opts = destinations(tab, id, matches!(a, Action::ScrollTo { .. }));
                    if let Some(g) = pick(
                        ui,
                        &format!("Interaction {n} destination"),
                        Some(&to),
                        &opts,
                    ) {
                        let action = match a {
                            Action::Navigate { .. } => Action::Navigate { to: g },
                            Action::Overlay { .. } => Action::Overlay { to: g },
                            Action::Swap { .. } => Action::Swap { to: g },
                            _ => Action::ScrollTo { to: g },
                        };
                        write(
                            tab,
                            ReactionSpec {
                                action,
                                ..r.clone()
                            },
                            "Interaction destination",
                        );
                    }
                }
            }
        }
    });
    ui.horizontal(|ui| {
        let transitions: Vec<(String, String)> = [
            ("instant", "Instant"),
            ("dissolve", "Dissolve"),
            ("smart_animate", "Smart animate"),
            ("move_in", "Move in"),
            ("push", "Push"),
            ("slide_in", "Slide in"),
        ]
        .iter()
        .map(|(k, l)| (k.to_string(), l.to_string()))
        .collect();
        let current = r.transition.clone().unwrap_or_else(|| "instant".into());
        if let Some(t) = pick(
            ui,
            &format!("Interaction {n} animation"),
            Some(&current),
            &transitions,
        ) {
            let duration = if t == "instant" {
                None
            } else {
                r.duration.or(Some(0.3))
            };
            write(
                tab,
                ReactionSpec {
                    transition: Some(t),
                    duration,
                    ..r.clone()
                },
                "Interaction animation",
            );
        }
        if current != "instant"
            && let Some(ms) = num_field(
                ui,
                "ms",
                &format!("Interaction {n} duration"),
                r.duration.map(|d| d * 1000.0),
                10.0,
                44.0,
            )
        {
            write(
                tab,
                ReactionSpec {
                    duration: Some((ms / 1000.0).max(0.0)),
                    ..r.clone()
                },
                "Interaction duration",
            );
        }
    });
}

/// The selected layer's connections to frames on this page, in screen points: between the
/// sides that face each other (its right edge to a destination's left edge, or its left edge
/// to the right edge of one further left). Scroll targets sit inside its own frame: not drawn.
pub(crate) fn connections(tab: &Tab, origin: Pos2) -> Vec<(Pos2, Pos2)> {
    let (Some(id), true) = (tab.selection, tab.also.is_empty()) else {
        return Vec::new();
    };
    let (Some(list), Some(from)) = (
        reactions_of(&tab.doc, id),
        figstudio_render::node_bounds(&tab.doc, id),
    ) else {
        return Vec::new();
    };
    let screen = |x: f64, y: f64| tab.view.to_screen(origin, pos2(x as f32, y as f32));
    list.iter()
        .filter(|r| !matches!(r.action, Action::ScrollTo { .. }))
        .filter_map(|r| destination(&r.action))
        .filter(|to| tab.doc.page_of(*to) == Some(tab.page))
        .filter_map(|to| figstudio_render::node_bounds(&tab.doc, to))
        .map(|b| {
            if b.center().x >= from.center().x {
                (screen(from.x1, from.center().y), screen(b.x0, b.center().y))
            } else {
                (screen(from.x0, from.center().y), screen(b.x1, b.center().y))
            }
        })
        .collect()
}

/// Draws [`connections`]: a curve with a dot at the start and an arrow at the end.
pub(crate) fn noodles(painter: &egui::Painter, tab: &Tab, origin: Pos2) {
    let stroke = Stroke::new(1.5, crate::theme::ACCENT);
    for (a, z) in connections(tab, origin) {
        let dir = if z.x >= a.x { 1.0 } else { -1.0 };
        let bend = ((z.x - a.x).abs() / 2.0).max(40.0) * dir;
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [a, a + vec2(bend, 0.0), z - vec2(bend, 0.0), z],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
        painter.circle_filled(a, 3.5, crate::theme::ACCENT);
        painter.add(egui::Shape::convex_polygon(
            vec![z, z + vec2(-8.0 * dir, -5.0), z + vec2(-8.0 * dir, 5.0)],
            crate::theme::ACCENT,
            Stroke::NONE,
        ));
    }
}
