//! Colours, gradients and bound colour variables as CSS values for Tailwind arbitrary classes.
use super::{Notes, num};
use figstudio_fig::{
    Val,
    tokens::{Tokens, Value},
};
use figstudio_mcp_core::variables::format::color as hex;
use figstudio_render::{geom::affine, kurbo::Point};

/// The visible paints of `key` (`fillPaints`/`strokePaints`), from an attached style when
/// one resolves (the node's own copy can be stale), bottom first as Figma stores them.
pub(super) fn paints<'v>(data: &'v Val, key: &str, tokens: &'v Tokens) -> Vec<&'v Val> {
    let style = figstudio_fig::resolve::STYLED_PAINTS
        .iter()
        .find(|(_, k)| *k == key)
        .and_then(|(style, _)| tokens.style(data.get(style)?)?.paints.as_ref());
    style
        .or_else(|| data.get(key))
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter(|p| p.flag("visible") != Some(false) && p.num("opacity") != Some(0.0))
        .collect()
}

/// A solid paint (or gradient stop) colour, as `var(--name,#hex)` when a variable is bound.
pub(super) fn color(paint: &Val, opacity: f32, tokens: &Tokens) -> Option<String> {
    let bound = paint.get("colorVar");
    let variable = bound
        .and_then(|v| v.path(&["value", "alias"]))
        .and_then(|alias| tokens.variable(alias));
    let resolved = bound.and_then(|v| tokens.resolve(v, None));
    let stored = match &resolved {
        Some(Value::Color(c)) => c,
        _ => paint.get("color")?,
    };
    let fallback = hex(stored, opacity)?;
    Some(match variable {
        // Paint opacity cannot be applied to a CSS variable; the fallback carries it.
        Some(v) if opacity >= 1.0 => format!("var(--{},{fallback})", css_name(&v.name)),
        _ => fallback,
    })
}

/// A variable name as a CSS custom property name: `Colors/Primary 500` → `colors-primary-500`.
pub(super) fn css_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// One paint layer as a CSS `background` image or colour; `None` when it is not a colour or
/// gradient (images are handled as assets).
pub(super) fn layer(
    paint: &Val,
    size: (f64, f64),
    tokens: &Tokens,
    notes: &mut Notes,
) -> Option<String> {
    let opacity = paint.num("opacity").unwrap_or(1.0);
    let kind = paint.text("type")?;
    if paint
        .text("blendMode")
        .is_some_and(|m| !matches!(m, "NORMAL" | "PASS_THROUGH"))
    {
        notes.add("paint blend modes dropped");
    }
    if kind == "SOLID" {
        return color(paint, opacity, tokens);
    }
    if !kind.starts_with("GRADIENT") {
        return None;
    }
    // Bound stops live in `stopsVar` (ColorStopVar); `stops` (ColorStop) is the plain copy.
    let stops: Vec<String> = paint
        .get("stopsVar")
        .filter(|v| !v.items().is_empty())
        .or_else(|| paint.get("stops"))
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter_map(|stop| {
            let c = color(stop, opacity, tokens)?;
            Some(format!(
                "{c} {}%",
                num(f64::from(stop.num("position")?.clamp(0.0, 1.0)) * 100.0)
            ))
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    let stops = stops.join(",");
    // The gradient runs from (0, .5) to (1, .5) in paint space; map it back to the node box.
    let inverse = affine(paint.get("transform")).inverse();
    let at = |x, y| {
        let p = inverse * Point::new(x, y);
        Point::new(p.x * size.0, p.y * size.1)
    };
    let (start, end) = (at(0.0, 0.5), at(1.0, 0.5));
    Some(match kind {
        "GRADIENT_LINEAR" => {
            let angle = (end.x - start.x).atan2(start.y - end.y).to_degrees();
            notes.add("gradient extents approximated");
            format!(
                "linear-gradient({}deg,{stops})",
                num(angle.rem_euclid(360.0))
            )
        }
        "GRADIENT_ANGULAR" => {
            let angle = (end.y - start.y).atan2(end.x - start.x).to_degrees() + 90.0;
            format!(
                "conic-gradient(from {}deg at {}% {}%,{stops})",
                num(angle.rem_euclid(360.0)),
                num(start.x / size.0.max(1e-6) * 100.0),
                num(start.y / size.1.max(1e-6) * 100.0)
            )
        }
        _ => {
            if kind == "GRADIENT_DIAMOND" {
                notes.add("diamond gradients approximated");
            }
            let radius = (end - start).hypot();
            let other = (at(0.5, 0.0) - start).hypot();
            format!(
                "radial-gradient({}px {}px at {}px {}px,{stops})",
                num(radius),
                num(other),
                num(start.x),
                num(start.y)
            )
        }
    })
}

/// The `background` classes for a node's fills: `bg-[#hex]` for one solid colour, otherwise an
/// arbitrary `background` property listing the layers top first (CSS order).
pub(super) fn background(layers: &[String]) -> Option<String> {
    match layers {
        [] => None,
        [one] if !one.contains("gradient(") => Some(format!("bg-[{}]", arbitrary(one))),
        _ => {
            let css: Vec<String> = layers
                .iter()
                .rev()
                .map(|l| {
                    if l.contains("gradient(") {
                        l.clone()
                    } else {
                        format!("linear-gradient({l},{l})")
                    }
                })
                .collect();
            Some(format!("[background:{}]", arbitrary(&css.join(","))))
        }
    }
}

/// A CSS value inside a Tailwind arbitrary class: Tailwind splits classes on whitespace and
/// reads `_` as a space, so spaces become underscores.
pub(super) fn arbitrary(css: &str) -> String {
    css.split_whitespace().collect::<Vec<_>>().join("_")
}
