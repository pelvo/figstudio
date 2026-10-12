//! Position, size and auto-layout (stack) fields as Tailwind classes.
use super::{Classes, Notes, num, px};
use figstudio_fig::Val;
use figstudio_render::{
    geom::{affine, clips_content, is_container, transformed_bounds},
    kurbo::Rect,
};

fn stack(data: &Val) -> Option<&str> {
    data.text("stackMode")
        .filter(|m| matches!(*m, "HORIZONTAL" | "VERTICAL"))
}

fn size(data: &Val) -> (f64, f64) {
    let axis = |a| {
        data.path(&["size", a])
            .and_then(Val::as_f32)
            .map_or(0.0, f64::from)
    };
    (axis("x"), axis("y"))
}

/// Lengths the renderer does not scale in a uniformly scaled instance either (only position,
/// size, strokes and stored glyphs are scaled): padding, gap, radii and text-style lengths.
pub(super) fn keeps_stored_lengths(data: &Val) -> bool {
    const LENGTHS: [&str; 13] = [
        "stackSpacing",
        "stackCounterSpacing",
        "stackPadding",
        "stackHorizontalPadding",
        "stackVerticalPadding",
        "stackPaddingRight",
        "stackPaddingBottom",
        "cornerRadius",
        "rectangleTopLeftCornerRadius",
        "rectangleTopRightCornerRadius",
        "rectangleBottomRightCornerRadius",
        "rectangleBottomLeftCornerRadius",
        "fontSize",
    ];
    LENGTHS
        .iter()
        .any(|key| data.num(key).is_some_and(|v| v.is_finite() && v != 0.0))
}

/// Whether the child is placed by its parent's auto-layout rather than by coordinates.
pub(super) fn in_flow(data: &Val, parent: Option<&Val>) -> bool {
    parent.is_some_and(|p| stack(p).is_some()) && data.text("stackPositioning") != Some("ABSOLUTE")
}

/// Placement and size classes. `parent` is `None` for the requested root; `baked` is set for
/// rendered assets, whose picture already holds rotation, so they use their bounding box.
pub(super) fn place(
    out: &mut Classes,
    kind: &str,
    data: &Val,
    parent: Option<&Val>,
    baked: bool,
    notes: &mut Notes,
) {
    if kind == "CANVAS" {
        return; // A page has no size; its children keep page coordinates.
    }
    let (w, h) = size(data);
    let transform = affine(data.get("transform"));
    let flow = in_flow(data, parent);
    let [a, b, c, d, x, y] = transform.as_coeffs();
    let (mut width, mut height) = (Some(w), Some(h));
    if baked {
        let aabb = transformed_bounds(transform, Rect::new(0.0, 0.0, w, h));
        (width, height) = (Some(aabb.width()), Some(aabb.height()));
        if parent.is_some() && !flow {
            out.push(format!(
                "absolute left-[{}] top-[{}]",
                px(aabb.x0),
                px(aabb.y0)
            ));
        }
    } else {
        if parent.is_some() && !flow {
            out.push(format!("absolute left-[{}] top-[{}]", px(x), px(y)));
        }
        let angle = b.atan2(a).to_degrees();
        if num(angle) != "0" {
            out.push(format!("origin-top-left rotate-[{}deg]", num(angle)));
        }
        if a * d - b * c < 0.0 {
            notes.add("mirrored layers not flipped");
        }
    }
    // Hugging auto-layout frames and auto-sized text take their size from their content.
    let horizontal = stack(data) == Some("HORIZONTAL");
    let hug = |field: &str| data.text(field) == Some("RESIZE_TO_FIT");
    if stack(data).is_some() {
        let (primary, counter) = (hug("stackPrimarySizing"), hug("stackCounterSizing"));
        if (horizontal && primary) || (!horizontal && counter) {
            width = None;
        }
        if (horizontal && counter) || (!horizontal && primary) {
            height = None;
        }
    }
    if kind == "TEXT" {
        match data.text("textAutoResize") {
            Some("WIDTH_AND_HEIGHT") => (width, height) = (None, None),
            Some("HEIGHT") => height = None,
            _ => {}
        }
    }
    if flow && let Some(parent) = parent {
        let row = stack(parent) == Some("HORIZONTAL");
        if data.num("stackChildPrimaryGrow").is_some_and(|g| g > 0.0) {
            out.push("grow");
            if row { width = None } else { height = None }
        }
        if data.text("stackChildAlignSelf") == Some("STRETCH") {
            out.push("self-stretch");
            if row { height = None } else { width = None }
        }
        out.push("shrink-0");
    }
    if let Some(w) = width {
        out.push(format!("w-[{}]", px(w)));
    }
    if let Some(h) = height {
        out.push(format!("h-[{}]", px(h)));
    }
}

/// Classes a container gives its children: flex, gap, padding and alignment, or `relative`
/// when children are placed by coordinates; `overflow-clip` when it clips them.
pub(super) fn container(
    out: &mut Classes,
    kind: &str,
    data: &Val,
    has_children: bool,
    notes: &mut Notes,
) {
    if data.text("stackMode") == Some("GRID") {
        notes.add("grid layouts placed absolutely");
    }
    let Some(mode) = stack(data) else {
        if has_children {
            out.relative();
        }
        if has_children && is_container(kind) && clips_content(kind, data) {
            out.push("overflow-clip");
        }
        return;
    };
    out.push(if mode == "VERTICAL" {
        "flex flex-col"
    } else {
        "flex"
    });
    if has_children {
        out.relative();
    }
    let justify = data
        .text("stackPrimaryAlignItems")
        .or_else(|| data.text("stackJustify"));
    let gap = data
        .num("stackSpacing")
        .filter(|g| g.is_finite() && *g != 0.0);
    let counter = data.num("stackCounterSpacing").filter(|g| g.is_finite());
    let wrap = data.text("stackWrap") == Some("WRAP");
    if wrap {
        out.push("flex-wrap");
        // Lines of a wrapped stack align as a group: `stackCounterAlignContent` AUTO (the
        // default) packs them at the start, SPACE_BETWEEN spreads them across the cross axis.
        match data.text("stackCounterAlignContent") {
            Some("SPACE_BETWEEN") => out.push("content-between"),
            _ => out.push("content-start"),
        }
    }
    match (wrap, counter) {
        (true, Some(c)) => {
            let (main, cross) = if mode == "VERTICAL" {
                ("y", "x")
            } else {
                ("x", "y")
            };
            if let Some(g) = gap.filter(|_| justify != Some("SPACE_BETWEEN")) {
                out.push(format!("gap-{main}-[{}]", px(f64::from(g))));
            }
            out.push(format!("gap-{cross}-[{}]", px(f64::from(c))));
        }
        _ => {
            if let Some(g) = gap.filter(|_| justify != Some("SPACE_BETWEEN")) {
                out.push(format!("gap-[{}]", px(f64::from(g))));
            }
        }
    }
    padding(out, data);
    match justify {
        Some("CENTER") => out.push("justify-center"),
        Some("MAX") => out.push("justify-end"),
        Some("SPACE_BETWEEN") => out.push("justify-between"),
        Some("SPACE_AROUND") => out.push("justify-around"),
        Some("SPACE_EVENLY" | "SPACE_EVENLY_CSS") => out.push("justify-evenly"),
        _ => {}
    }
    match data
        .text("stackCounterAlignItems")
        .or_else(|| data.text("stackCounterAlign"))
    {
        Some("CENTER") => out.push("items-center"),
        Some("MAX") => out.push("items-end"),
        Some("BASELINE") => out.push("items-baseline"),
        Some("STRETCH") => {}
        _ => out.push("items-start"),
    }
    if has_children && clips_content(kind, data) {
        out.push("overflow-clip");
    }
}

/// Left and top come from the axis fields; right and bottom fall back to them.
fn padding(out: &mut Classes, data: &Val) {
    let legacy = data.num("stackPadding");
    let field = |name: &str| data.num(name).filter(|v| v.is_finite());
    let left = field("stackHorizontalPadding").or(legacy).unwrap_or(0.0);
    let top = field("stackVerticalPadding").or(legacy).unwrap_or(0.0);
    let right = field("stackPaddingRight").unwrap_or(left);
    let bottom = field("stackPaddingBottom").unwrap_or(top);
    let side = |prefix: &str, v: f32, out: &mut Classes| {
        if v != 0.0 {
            out.push(format!("{prefix}-[{}]", px(f64::from(v))));
        }
    };
    if left == right && top == bottom {
        if left == top {
            side("p", left, out);
        } else {
            side("px", left, out);
            side("py", top, out);
        }
    } else {
        side("pt", top, out);
        side("pr", right, out);
        side("pb", bottom, out);
        side("pl", left, out);
    }
}
