//! Edit commands on the selection, shared by the Edit menu, the right-click menu and shortcuts.
//! Each builds one `EditOp` batch, so it is one undo step.

use figstudio_fig::Guid;
use figstudio_fig::edit::vector_net::BooleanOp;
use figstudio_fig::edit::{AlignEdge, AlignTo, Axis, EditOp, Pivot, ZOrder};

use crate::tab::Tab;

use super::apply_tab_ops;

/// One edit command. Everything acts on the whole selection unless it says otherwise.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum EditCmd {
    Group,
    Ungroup,
    FrameSelection,
    Reorder(ZOrder),
    Flip(Axis),
    /// Rotate 90° clockwise (`true`) or counter-clockwise, about each layer's centre.
    Rotate90(bool),
    Align(AlignEdge),
    Distribute(Axis),
    ToggleVisible,
    ToggleLocked,
    CreateComponent,
    DetachInstance,
    Boolean(BooleanOp),
    Flatten,
    OutlineStroke,
    UseAsMask,
}

impl EditCmd {
    /// The menu label.
    pub(crate) fn label(self) -> &'static str {
        use EditCmd::*;
        match self {
            Group => "Group selection",
            Ungroup => "Ungroup",
            FrameSelection => "Frame selection",
            Reorder(ZOrder::Front) => "Bring to front",
            Reorder(ZOrder::Forward) => "Bring forward",
            Reorder(ZOrder::Backward) => "Send backward",
            Reorder(ZOrder::Back) => "Send to back",
            Flip(Axis::Horizontal) => "Flip horizontal",
            Flip(Axis::Vertical) => "Flip vertical",
            Rotate90(true) => "Rotate 90° right",
            Rotate90(false) => "Rotate 90° left",
            Align(AlignEdge::Left) => "Align left",
            Align(AlignEdge::HCenter) => "Align horizontal centers",
            Align(AlignEdge::Right) => "Align right",
            Align(AlignEdge::Top) => "Align top",
            Align(AlignEdge::VCenter) => "Align vertical centers",
            Align(AlignEdge::Bottom) => "Align bottom",
            Distribute(Axis::Horizontal) => "Distribute horizontal spacing",
            Distribute(Axis::Vertical) => "Distribute vertical spacing",
            ToggleVisible => "Show/Hide",
            ToggleLocked => "Lock/Unlock",
            CreateComponent => "Create component",
            DetachInstance => "Detach instance",
            Boolean(BooleanOp::Union) => "Union selection",
            Boolean(BooleanOp::Subtract) => "Subtract selection",
            Boolean(BooleanOp::Intersect) => "Intersect selection",
            Boolean(BooleanOp::Exclude) => "Exclude selection",
            Flatten => "Flatten",
            OutlineStroke => "Outline stroke",
            UseAsMask => "Use as mask",
        }
    }

    /// Whether the command can run on `tab`'s selection (menus show the rest disabled).
    pub(crate) fn enabled(self, tab: &Tab) -> bool {
        use EditCmd::*;
        let sel = selection(tab);
        let kinds = || {
            sel.iter()
                .filter_map(|id| tab.doc.node(*id).map(|n| n.kind.as_str()))
        };
        let siblings = sel.len() >= 2 && same_parent(tab, &sel);
        match self {
            Group | FrameSelection | Flatten => !sel.is_empty() && same_parent(tab, &sel),
            Boolean(_) => siblings,
            Distribute(_) => sel.len() >= 3,
            Ungroup => sel.iter().any(|id| {
                tab.doc
                    .node(*id)
                    .is_some_and(figstudio_fig::edit::groups::is_group)
            }),
            CreateComponent => !sel.is_empty() && kinds().all(|k| k == "FRAME"),
            DetachInstance => kinds().any(|k| k == "INSTANCE"),
            OutlineStroke => sel.iter().any(|id| {
                tab.doc.node(*id).is_some_and(|n| {
                    n.data
                        .get("strokePaints")
                        .is_some_and(|p| !p.items().is_empty())
                })
            }),
            UseAsMask => sel.len() == 1,
            _ => !sel.is_empty(),
        }
    }
}

/// The selected layers, pages left out.
fn selection(tab: &Tab) -> Vec<Guid> {
    tab.selected()
        .into_iter()
        .filter(|&id| id != tab.page && tab.doc.node(id).is_some())
        .collect()
}

fn same_parent(tab: &Tab, ids: &[Guid]) -> bool {
    let parent = |id: &Guid| tab.doc.node(*id).and_then(|n| n.parent);
    ids.first()
        .is_some_and(|f| ids.iter().all(|id| parent(id) == parent(f)))
}

/// Runs `cmd` on `tab`'s selection; returns the toast text.
pub(crate) fn run(tab: &mut Tab, cmd: EditCmd) -> Result<String, String> {
    use EditCmd::*;
    let sel = selection(tab);
    if sel.is_empty() {
        return Err("Select a layer first".into());
    }
    if !cmd.enabled(tab) {
        return Err(format!("{} does not apply to this selection", cmd.label()));
    }
    let each = |f: &dyn Fn(Guid) -> EditOp| sel.iter().map(|&id| f(id)).collect::<Vec<_>>();
    let ops: Vec<EditOp> = match cmd {
        Group => vec![EditOp::Group {
            targets: sel.clone(),
            name: None,
        }],
        FrameSelection => vec![EditOp::FrameSelection {
            targets: sel.clone(),
            name: None,
        }],
        Ungroup => sel
            .iter()
            .filter(|id| {
                tab.doc
                    .node(**id)
                    .is_some_and(figstudio_fig::edit::groups::is_group)
            })
            .map(|&target| EditOp::Ungroup { target })
            .collect(),
        // The topmost layer moves first toward the front (and the bottom one toward the back),
        // so several layers keep their order among themselves.
        Reorder(to) => {
            let mut ordered = sel.clone();
            ordered.sort_by_key(|id| z_index(tab, *id));
            if matches!(to, ZOrder::Front | ZOrder::Forward) {
                ordered.reverse();
            }
            ordered
                .into_iter()
                .map(|target| EditOp::Reorder { target, to })
                .collect()
        }
        Flip(axis) => each(&|target| EditOp::Flip { target, axis }),
        Rotate90(clockwise) => each(&|target| EditOp::Rotate {
            target,
            degrees: current_rotation(tab, target) + if clockwise { 90.0 } else { -90.0 },
            pivot: Pivot::Center,
        }),
        Align(edge) => vec![EditOp::Align {
            targets: sel.clone(),
            edge,
            to: AlignTo::Selection,
        }],
        Distribute(axis) => vec![EditOp::Distribute {
            targets: sel.clone(),
            axis,
            spacing: None,
        }],
        // One click sets them all alike: hidden if any is shown (as Figma does), else shown.
        ToggleVisible => {
            let hide = sel.iter().any(|id| flag(tab, *id, "visible", true));
            each(&|target| layer_props(target, Some(!hide), None))
        }
        ToggleLocked => {
            let lock = sel.iter().any(|id| !flag(tab, *id, "locked", false));
            each(&|target| layer_props(target, None, Some(lock)))
        }
        CreateComponent => each(&|frame| EditOp::CreateComponent { frame }),
        DetachInstance => sel
            .iter()
            .filter(|id| tab.doc.node(**id).is_some_and(|n| n.kind == "INSTANCE"))
            .map(|&instance| EditOp::DetachInstance { instance })
            .collect(),
        Boolean(operation) => vec![EditOp::BooleanCombine {
            operation,
            targets: sel.clone(),
            name: None,
        }],
        Flatten => vec![EditOp::Flatten {
            targets: sel.clone(),
            name: None,
        }],
        OutlineStroke => sel
            .iter()
            .filter(|id| {
                tab.doc.node(**id).is_some_and(|n| {
                    n.data
                        .get("strokePaints")
                        .is_some_and(|p| !p.items().is_empty())
                })
            })
            .map(|&target| EditOp::OutlineStroke { target })
            .collect(),
        UseAsMask => {
            let on = !flag(tab, sel[0], "mask", false);
            vec![EditOp::SetMask {
                target: sel[0],
                mask: on.then_some(figstudio_fig::edit::MaskKind::Alpha),
            }]
        }
    };
    // Ungrouped children are selected afterwards, as in Figma.
    let freed: Vec<Guid> = if cmd == Ungroup {
        ops.iter()
            .filter_map(|op| match op {
                EditOp::Ungroup { target } => Some(tab.doc.children(*target).to_vec()),
                _ => None,
            })
            .flatten()
            .collect()
    } else {
        Vec::new()
    };
    let made = apply_tab_ops(tab, &ops, Some(cmd.label()), None)?;
    // Commands that make a layer select it; the rest keep the selection.
    if matches!(
        cmd,
        Group | FrameSelection | Boolean(_) | Flatten | OutlineStroke | DetachInstance
    ) {
        tab.select_all_of(&made);
    } else if cmd == Ungroup {
        tab.select_all_of(&freed);
    }
    Ok(cmd.label().to_string())
}

fn z_index(tab: &Tab, id: Guid) -> usize {
    tab.doc
        .node(id)
        .and_then(|n| n.parent)
        .and_then(|p| tab.doc.children(p).iter().position(|c| *c == id))
        .unwrap_or(0)
}

fn flag(tab: &Tab, id: Guid, field: &str, default: bool) -> bool {
    tab.doc
        .node(id)
        .and_then(|n| n.data.flag(field))
        .unwrap_or(default)
}

fn layer_props(target: Guid, visible: Option<bool>, locked: Option<bool>) -> EditOp {
    EditOp::SetLayerProps {
        target,
        visible,
        locked,
        opacity: None,
        blend_mode: None,
        clip_content: None,
        proportions_locked: None,
    }
}

/// The layer's rotation in degrees, in `matrix_2x3`'s convention (y points down, so positive
/// turns clockwise on screen).
fn current_rotation(tab: &Tab, id: Guid) -> f64 {
    current_rotation_of(&tab.doc, id)
}

/// A layer's rotation in degrees (positive turns clockwise on screen).
pub(crate) fn current_rotation_of(doc: &figstudio_fig::Document, id: Guid) -> f64 {
    let Some(n) = doc.node(id) else {
        return 0.0;
    };
    let t = figstudio_fig::edit::space::Xform::of(&n.data);
    t.m10.atan2(t.m00).to_degrees()
}
