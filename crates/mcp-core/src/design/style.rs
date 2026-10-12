//! Strokes, corner radii, effects, opacity and blend mode as Tailwind classes.
use super::{Classes, Notes, num, paint, px};
use figstudio_fig::{Val, tokens::Tokens};

/// Box decoration for a non-asset layer (assets carry these in their picture).
pub(super) fn decorate(
    out: &mut Classes,
    kind: &str,
    data: &Val,
    tokens: &Tokens,
    notes: &mut Notes,
) {
    if kind != "TEXT" {
        border(out, data, tokens, notes);
        radius(out, data);
    }
    effects(out, kind == "TEXT", data, tokens, notes);
    if let Some(opacity) = data.num("opacity").filter(|o| *o < 1.0) {
        out.push(format!("opacity-[{}]", num(f64::from(opacity))));
    }
    common(out, data, notes);
}

/// Blend mode and masks: what a rendered asset cannot carry, because they depend on the
/// layers behind it. Opacity and effects are already baked into the asset's picture.
pub(super) fn common(out: &mut Classes, data: &Val, notes: &mut Notes) {
    if let Some(mode) = data
        .text("blendMode")
        .filter(|m| !matches!(*m, "NORMAL" | "PASS_THROUGH"))
    {
        let css = match mode {
            "LINEAR_DODGE" => Some("plus-lighter".to_owned()),
            "LINEAR_BURN" => None,
            other => Some(other.to_ascii_lowercase().replace('_', "-")),
        };
        match css {
            Some(css) => out.push(format!("mix-blend-{css}")),
            None => notes.add("blend modes approximated"),
        }
    }
    if data.flag("mask") == Some(true) {
        notes.add("masks not applied");
    }
}

fn border(out: &mut Classes, data: &Val, tokens: &Tokens, notes: &mut Notes) {
    let paints = paint::paints(data, "strokePaints", tokens);
    let Some(weight) = data.num("strokeWeight").filter(|w| *w > 0.0) else {
        return;
    };
    let Some(first) = paints.last() else { return };
    if paints.len() > 1 {
        notes.add("extra stroke paints dropped");
    }
    let Some(color) = (first.text("type") == Some("SOLID"))
        .then(|| paint::color(first, first.num("opacity").unwrap_or(1.0), tokens))
        .flatten()
    else {
        notes.add("non-solid strokes dropped");
        return;
    };
    if data.flag("borderStrokeWeightsIndependent") == Some(true) {
        for (side, field) in [
            ("t", "borderTopWeight"),
            ("r", "borderRightWeight"),
            ("b", "borderBottomWeight"),
            ("l", "borderLeftWeight"),
        ] {
            let w = data.num(field).unwrap_or(0.0);
            if w > 0.0 {
                out.push(format!("border-{side}-[{}]", px(f64::from(w))));
            }
        }
    } else {
        out.push(format!("border-[{}]", px(f64::from(weight))));
    }
    out.push("border-solid");
    out.push(format!("border-[color:{color}]"));
    if !data
        .get("dashPattern")
        .map(Val::items)
        .unwrap_or_default()
        .is_empty()
    {
        out.push("border-dashed");
    }
    // CSS borders sit inside the box like Figma's INSIDE strokes; other alignments differ.
    if data.text("strokeAlign").is_some_and(|a| a != "INSIDE") {
        notes.add("stroke alignments approximated");
    }
}

fn radius(out: &mut Classes, data: &Val) {
    let corners = if data.flag("rectangleCornerRadiiIndependent") == Some(true) {
        [
            "rectangleTopLeftCornerRadius",
            "rectangleTopRightCornerRadius",
            "rectangleBottomRightCornerRadius",
            "rectangleBottomLeftCornerRadius",
        ]
        .map(|f| data.num(f).unwrap_or(0.0))
    } else {
        [data.num("cornerRadius").unwrap_or(0.0); 4]
    };
    if corners.iter().all(|r| *r == corners[0]) {
        if corners[0] > 0.0 {
            out.push(format!("rounded-[{}]", px(f64::from(corners[0]))));
        }
        return;
    }
    for (side, r) in ["tl", "tr", "br", "bl"].into_iter().zip(corners) {
        if r > 0.0 {
            out.push(format!("rounded-{side}-[{}]", px(f64::from(r))));
        }
    }
}

fn effects(out: &mut Classes, text: bool, data: &Val, tokens: &Tokens, notes: &mut Notes) {
    let mut shadows = Vec::new();
    // An attached effect style is the source of truth; the node's copy can be stale.
    let styled = data
        .get("styleIdForEffect")
        .or_else(|| data.get("inheritEffectStyleID"))
        .and_then(|r| tokens.style(r))
        .and_then(|s| s.effects.as_ref());
    for effect in styled
        .or_else(|| data.get("effects"))
        .map(Val::items)
        .unwrap_or_default()
    {
        if effect.flag("visible") == Some(false) {
            continue;
        }
        let radius = f64::from(effect.num("radius").unwrap_or(0.0).max(0.0));
        let kind = effect.text("type").unwrap_or("");
        match kind {
            "DROP_SHADOW" | "INNER_SHADOW" => {
                let color = effect
                    .get("color")
                    .and_then(|c| crate::variables::format::color(c, 1.0))
                    .unwrap_or_else(|| "#00000040".into());
                let offset = |a| {
                    f64::from(
                        effect
                            .path(&["offset", a])
                            .and_then(Val::as_f32)
                            .unwrap_or(0.0),
                    )
                };
                let spread = f64::from(effect.num("spread").unwrap_or(0.0));
                let inset = if kind == "INNER_SHADOW" { "inset " } else { "" };
                if text && (!inset.is_empty() || spread != 0.0) {
                    notes.add("text shadows approximated");
                }
                shadows.push(if text {
                    format!(
                        "{} {} {} {color}",
                        px(offset("x")),
                        px(offset("y")),
                        px(radius)
                    )
                } else {
                    format!(
                        "{inset}{} {} {} {} {color}",
                        px(offset("x")),
                        px(offset("y")),
                        px(radius),
                        px(spread)
                    )
                });
            }
            "FOREGROUND_BLUR" => out.push(format!("blur-[{}]", px(radius / 2.0))),
            "BACKGROUND_BLUR" => out.push(format!("backdrop-blur-[{}]", px(radius / 2.0))),
            _ => notes.add("effects skipped"),
        }
    }
    if !shadows.is_empty() {
        let property = if text { "[text-shadow:" } else { "shadow-[" };
        out.push(format!(
            "{property}{}]",
            shadows.join(",").replace(' ', "_")
        ));
    }
}
