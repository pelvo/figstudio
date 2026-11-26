//! Masks, layout grids, export settings and prototype [`EditOp`]s.

use serde::{Deserialize, Serialize};

use crate::edit::specs::color_val;
use crate::edit::{GuidAlloc, Txn};
use crate::{Document, FigError, Guid, Obj, Val, intern};

use super::EditOp;
use super::compile_tree::ensure_exists;

/// How a mask layer masks the layers above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaskKind {
    /// By the mask's opacity.
    Alpha,
    /// By the mask's shape, ignoring its fill.
    Outline,
    /// By the mask's brightness.
    Luminance,
}

/// A layout grid on a frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "pattern", rename_all = "snake_case")]
pub enum GridSpec {
    /// A square grid of `size` cells.
    Grid {
        #[serde(default = "default_cell")]
        size: f64,
        #[serde(default)]
        hex: Option<String>,
        #[serde(default)]
        visible: Option<bool>,
    },
    Columns(StripeSpec),
    Rows(StripeSpec),
}

fn default_cell() -> f64 {
    10.0
}

/// Columns or rows of a layout grid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StripeSpec {
    #[serde(default = "default_count")]
    pub count: u32,
    /// `min`, `center`, `stretch` (default) or `max`.
    #[serde(default)]
    pub align: Option<String>,
    #[serde(default)]
    pub gutter: Option<f64>,
    /// Margin (stretch) or offset (min, max).
    #[serde(default)]
    pub offset: Option<f64>,
    /// Column or row width (not used when stretching).
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub hex: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
}

fn default_count() -> u32 {
    5
}

/// An export setting on a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSpec {
    /// `png`, `jpg`, `svg` or `pdf`.
    pub format: String,
    /// Scale (`2` = @2x); or give `width` or `height` instead.
    #[serde(default)]
    pub scale: Option<f64>,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub suffix: Option<String>,
}

/// What starts a prototype reaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    OnClick,
    OnHover,
    OnPress,
    OnDrag,
    MouseEnter,
    MouseLeave,
    MouseDown,
    MouseUp,
    /// After `ms` milliseconds on the screen.
    AfterTimeout {
        ms: f64,
    },
}

/// What a prototype reaction does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Navigate {
        #[serde(with = "crate::edit::guid_wire")]
        to: Guid,
    },
    Overlay {
        #[serde(with = "crate::edit::guid_wire")]
        to: Guid,
    },
    Swap {
        #[serde(with = "crate::edit::guid_wire")]
        to: Guid,
    },
    ScrollTo {
        #[serde(with = "crate::edit::guid_wire")]
        to: Guid,
    },
    Back,
    Close,
    Url {
        url: String,
    },
}

/// One prototype reaction: a trigger, an action and its transition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReactionSpec {
    pub trigger: Trigger,
    pub action: Action,
    /// `instant` (default), `dissolve`, `smart_animate`, `move_in`, `push`, `slide_in`, …: any
    /// Figma transition name in snake case (`slide_from_left`, `push_from_right`, …).
    #[serde(default)]
    pub transition: Option<String>,
    /// Seconds.
    #[serde(default)]
    pub duration: Option<f64>,
    /// `linear`, `ease_in`, `ease_out` (`out_cubic`), `ease_in_and_out` (`inout_cubic`), …
    #[serde(default)]
    pub easing: Option<String>,
}

pub(super) fn compile_proto_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::SetMask { target, mask } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            match mask {
                Some(kind) => {
                    set.insert("mask", Val::Bool(true));
                    let (t, outline) = match kind {
                        MaskKind::Alpha => ("ALPHA", false),
                        MaskKind::Outline => ("OUTLINE", true),
                        MaskKind::Luminance => ("LUMINANCE", false),
                    };
                    set.insert("maskType", Val::Enum(intern(t)));
                    set.insert("maskIsOutline", Val::Bool(outline));
                }
                None => {
                    set.insert("mask", Val::Bool(false));
                    for f in ["maskType", "maskIsOutline"] {
                        if node.data.get(f).is_some() {
                            unset.push(f.to_string());
                        }
                    }
                }
            }
            txn.modify(*target, set, unset);
            if let Some(p) = node.parent {
                layout_touched.push(p);
            }
        }
        EditOp::SetLayoutGrids { target, grids } => {
            let node = ensure_exists(doc, *target)?;
            if !matches!(node.kind.as_str(), "FRAME" | "SYMBOL" | "INSTANCE") {
                return Err(FigError::malformed(format!(
                    "{target} is {}; layout grids go on frames",
                    node.kind
                )));
            }
            let list = grids.iter().map(grid_val).collect::<Result<Vec<_>, _>>()?;
            let unset = drop_if(node.data.get("styleIdForGrid").is_some(), "styleIdForGrid");
            let mut set = Obj::new();
            set.insert("layoutGrids", Val::Arr(list));
            txn.modify(*target, set, unset);
        }
        EditOp::SetExportSettings { target, settings } => {
            ensure_exists(doc, *target)?;
            let list = settings
                .iter()
                .map(export_val)
                .collect::<Result<Vec<_>, _>>()?;
            txn.set_field(*target, "exportSettings", Val::Arr(list));
        }
        EditOp::SetReactions { target, reactions } => {
            ensure_exists(doc, *target)?;
            let mut list = Vec::with_capacity(reactions.len());
            for r in reactions {
                list.push(reaction_val(doc, alloc, r)?);
            }
            txn.set_field(*target, "prototypeInteractions", Val::Arr(list));
        }
        EditOp::SetFlowStart { frame, name } => {
            let node = ensure_exists(doc, *frame)?;
            let on_page = node
                .parent
                .and_then(|p| doc.node(p))
                .is_some_and(|p| p.kind == "CANVAS");
            if !on_page {
                return Err(FigError::malformed(format!(
                    "{frame}: a flow starts at a top-level frame"
                )));
            }
            match name {
                Some(n) => txn.set_field(
                    *frame,
                    "prototypeStartingPoint",
                    Val::Obj(Obj::from([("name", Val::Str(n.clone()))])),
                ),
                None => txn.unset_field(*frame, "prototypeStartingPoint"),
            }
        }
        EditOp::SetScrollBehavior {
            target,
            overflow,
            behavior,
        } => {
            ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if let Some(o) = overflow {
                set.insert(
                    "scrollDirection",
                    Val::Enum(intern(&o.to_ascii_uppercase())),
                );
            }
            if let Some(b) = behavior {
                let stored = match b.as_str() {
                    "scrolls" => "SCROLLS",
                    "fixed" => "FIXED_WHEN_CHILD_OF_SCROLLING_FRAME",
                    "sticky" => "STICKY_SCROLLS",
                    other => {
                        return Err(FigError::malformed(format!(
                            "unknown scroll behavior {other:?} (scrolls, fixed or sticky)"
                        )));
                    }
                };
                set.insert("scrollBehavior", Val::Enum(intern(stored)));
            }
            txn.modify(*target, set, Vec::new());
        }
        _ => {
            return Err(FigError::malformed(
                "not a mask, grid, export or prototype op",
            ));
        }
    }
    Ok(())
}

fn drop_if(cond: bool, field: &str) -> Vec<String> {
    if cond {
        vec![field.to_string()]
    } else {
        Vec::new()
    }
}

fn nonneg(v: f64, what: &str) -> Result<Val, FigError> {
    if v.is_finite() && v >= 0.0 {
        Ok(Val::Float(v))
    } else {
        Err(FigError::malformed(format!(
            "{what} must be a finite, non-negative number"
        )))
    }
}

fn grid_val(g: &GridSpec) -> Result<Val, FigError> {
    // Figma's default grid colours: red at 10% (grid, columns) or 10% red rows.
    let default_hex = "#FF00001A";
    let mut o = Obj::new();
    match g {
        GridSpec::Grid { size, hex, visible } => {
            o.insert("type", Val::Enum(intern("STRETCH")));
            o.insert("axis", Val::Enum(intern("X")));
            o.insert("pattern", Val::Enum(intern("GRID")));
            o.insert("sectionSize", nonneg(*size, "grid size")?);
            o.insert("numSections", Val::Int(1));
            o.insert("offset", Val::Float(0.0));
            o.insert("gutterSize", Val::Float(0.0));
            o.insert("color", color_val(hex.as_deref().unwrap_or(default_hex))?);
            o.insert("visible", Val::Bool(visible.unwrap_or(true)));
        }
        GridSpec::Columns(s) | GridSpec::Rows(s) => {
            let axis = if matches!(g, GridSpec::Columns(_)) {
                "X"
            } else {
                "Y"
            };
            let align = s.align.as_deref().unwrap_or("stretch").to_ascii_uppercase();
            if !matches!(align.as_str(), "MIN" | "CENTER" | "STRETCH" | "MAX") {
                return Err(FigError::malformed(format!(
                    "unknown grid align {align:?} (min, center, stretch or max)"
                )));
            }
            o.insert("type", Val::Enum(intern(&align)));
            o.insert("axis", Val::Enum(intern(axis)));
            o.insert("pattern", Val::Enum(intern("STRIPES")));
            o.insert("numSections", Val::Int(i64::from(s.count.clamp(1, 1000))));
            o.insert("gutterSize", nonneg(s.gutter.unwrap_or(20.0), "gutter")?);
            o.insert("offset", nonneg(s.offset.unwrap_or(0.0), "offset")?);
            o.insert("sectionSize", nonneg(s.size.unwrap_or(0.0), "size")?);
            o.insert("color", color_val(s.hex.as_deref().unwrap_or(default_hex))?);
            o.insert("visible", Val::Bool(s.visible.unwrap_or(true)));
        }
    }
    Ok(Val::Obj(o))
}

fn export_val(e: &ExportSpec) -> Result<Val, FigError> {
    let fmt = match e.format.to_ascii_lowercase().as_str() {
        "png" => "PNG",
        "jpg" | "jpeg" => "JPEG",
        "svg" => "SVG",
        "pdf" => "PDF",
        other => {
            return Err(FigError::malformed(format!(
                "unknown export format {other:?} (png, jpg, svg or pdf)"
            )));
        }
    };
    let (kind, value) = match (e.scale, e.width, e.height) {
        (Some(s), None, None) => ("CONTENT_SCALE", s),
        (None, Some(w), None) => ("CONTENT_WIDTH", w),
        (None, None, Some(h)) => ("CONTENT_HEIGHT", h),
        (None, None, None) => ("CONTENT_SCALE", 1.0),
        _ => {
            return Err(FigError::malformed(
                "give one of scale, width or height for an export",
            ));
        }
    };
    if !(value.is_finite() && value > 0.0) {
        return Err(FigError::malformed("an export size must be positive"));
    }
    Ok(Val::Obj(Obj::from([
        ("suffix", Val::Str(e.suffix.clone().unwrap_or_default())),
        ("imageType", Val::Enum(intern(fmt))),
        (
            "constraint",
            Val::Obj(Obj::from([
                ("type", Val::Enum(intern(kind))),
                ("value", Val::Float(value)),
            ])),
        ),
        ("contentsOnly", Val::Bool(true)),
        ("useAbsoluteBounds", Val::Bool(false)),
    ])))
}

fn reaction_val(doc: &Document, alloc: &mut GuidAlloc, r: &ReactionSpec) -> Result<Val, FigError> {
    let mut event = Obj::new();
    let trigger = match &r.trigger {
        Trigger::OnClick => "ON_CLICK",
        Trigger::OnHover => "ON_HOVER",
        Trigger::OnPress => "ON_PRESS",
        Trigger::OnDrag => "DRAG",
        Trigger::MouseEnter => "MOUSE_ENTER",
        Trigger::MouseLeave => "MOUSE_LEAVE",
        Trigger::MouseDown => "MOUSE_DOWN",
        Trigger::MouseUp => "MOUSE_UP",
        Trigger::AfterTimeout { ms } => {
            if !(ms.is_finite() && *ms >= 0.0) {
                return Err(FigError::malformed("a timeout must be non-negative"));
            }
            event.insert("transitionTimeout", Val::Float(ms / 1000.0));
            "AFTER_TIMEOUT"
        }
    };
    event.insert("interactionType", Val::Enum(intern(trigger)));
    let mut action = Obj::new();
    let target = |to: &Guid| -> Result<Val, FigError> {
        doc.node(*to)
            .map(|_| to.to_val())
            .ok_or_else(|| FigError::malformed(format!("destination {to} does not exist")))
    };
    let (connection, navigation) = match &r.action {
        Action::Navigate { to } => {
            action.insert("transitionNodeID", target(to)?);
            ("INTERNAL_NODE", Some("NAVIGATE"))
        }
        Action::Overlay { to } => {
            action.insert("transitionNodeID", target(to)?);
            ("INTERNAL_NODE", Some("OVERLAY"))
        }
        Action::Swap { to } => {
            action.insert("transitionNodeID", target(to)?);
            ("INTERNAL_NODE", Some("SWAP"))
        }
        Action::ScrollTo { to } => {
            action.insert("transitionNodeID", target(to)?);
            ("INTERNAL_NODE", Some("SCROLL_TO"))
        }
        Action::Back => ("BACK", None),
        Action::Close => ("CLOSE", None),
        Action::Url { url } => {
            action.insert("connectionURL", Val::Str(url.clone()));
            ("URL", None)
        }
    };
    action.insert("connectionType", Val::Enum(intern(connection)));
    if let Some(n) = navigation {
        action.insert("navigationType", Val::Enum(intern(n)));
    }
    let transition = r
        .transition
        .as_deref()
        .unwrap_or("instant")
        .to_ascii_uppercase();
    let transition = match transition.as_str() {
        "INSTANT" => "INSTANT_TRANSITION".to_string(),
        other => other.to_string(),
    };
    action.insert("transitionType", Val::Enum(intern(&transition)));
    if let Some(d) = r.duration {
        action.insert("transitionDuration", nonneg(d, "duration")?);
    }
    if let Some(e) = &r.easing {
        let easing = match e.to_ascii_lowercase().as_str() {
            "ease_in" => "IN_CUBIC".to_string(),
            "ease_out" => "OUT_CUBIC".to_string(),
            "ease_in_and_out" => "INOUT_CUBIC".to_string(),
            other => other.to_ascii_uppercase(),
        };
        action.insert("easingType", Val::Enum(intern(&easing)));
    }
    Ok(Val::Obj(Obj::from([
        ("id", alloc.next_guid().to_val()),
        ("event", Val::Obj(event)),
        ("actions", Val::Arr(vec![Val::Obj(action)])),
        ("isDeleted", Val::Bool(false)),
    ])))
}
