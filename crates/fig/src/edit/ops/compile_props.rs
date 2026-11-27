//! Appearance, Auto-Layout, constraint, and typography [`EditOp`] compilation.

use crate::edit::schema_check::{intern_node_enum, intern_node_enum_path, require_finite_f32};
use crate::edit::styles_vars::{
    blur_effect, hex_paint, image_paint_with_scale_mode, shadow_effect,
};
use crate::edit::text_runs::{
    apply_text_range_style_with_schema, commit_text_data, set_text_plain, splice_text_utf16,
    update_auto_resize_box,
};
use crate::edit::{GuidAlloc, Txn};
use crate::{Document, FigError, Guid, Obj, Val, intern};

use super::EditOp;
use super::compile_components::compile_component_or_vector_op;
use super::compile_tree::ensure_exists;

pub(super) fn compile_prop_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::SetCornerRadii {
            target,
            radii,
            smoothing,
        } => {
            ensure_exists(doc, *target)?;
            for r in radii {
                require_finite_f32(f64::from(*r), "set_corner_radii radii")?;
            }
            if let Some(sm) = smoothing {
                require_finite_f32(f64::from(*sm), "set_corner_radii smoothing")?;
            }
            let uniform = (radii[0] - radii[1]).abs() < 1e-4
                && (radii[1] - radii[2]).abs() < 1e-4
                && (radii[2] - radii[3]).abs() < 1e-4;
            let mut set = Obj::new();
            set.insert("cornerRadius", Val::Float(f64::from(radii[0].max(0.0))));
            set.insert("rectangleCornerRadiiIndependent", Val::Bool(!uniform));
            set.insert(
                "rectangleTopLeftCornerRadius",
                Val::Float(f64::from(radii[0].max(0.0))),
            );
            set.insert(
                "rectangleTopRightCornerRadius",
                Val::Float(f64::from(radii[1].max(0.0))),
            );
            set.insert(
                "rectangleBottomRightCornerRadius",
                Val::Float(f64::from(radii[2].max(0.0))),
            );
            set.insert(
                "rectangleBottomLeftCornerRadius",
                Val::Float(f64::from(radii[3].max(0.0))),
            );
            if let Some(sm) = smoothing {
                set.insert("cornerSmoothing", Val::Float(f64::from(sm.clamp(0.0, 1.0))));
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::SetConstraints {
            target,
            horizontal,
            vertical,
        } => {
            ensure_exists(doc, *target)?;
            let schema = doc.decoded_schema()?;
            let at = format!("node {target}");
            let mut set = Obj::new();
            if let Some(h) = horizontal {
                set.insert(
                    "horizontalConstraint",
                    intern_node_enum(&schema, "horizontalConstraint", h, &at)?,
                );
            }
            if let Some(v) = vertical {
                set.insert(
                    "verticalConstraint",
                    intern_node_enum(&schema, "verticalConstraint", v, &at)?,
                );
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::SetAutoLayout {
            target,
            mode,
            spacing,
            counter_spacing,
            padding,
            primary_sizing,
            counter_sizing,
            primary_align,
            counter_align,
            wrap,
        } => {
            ensure_exists(doc, *target)?;
            let schema = doc.decoded_schema()?;
            let at = format!("node {target}");
            let mut set = Obj::new();
            set.insert(
                "stackMode",
                intern_node_enum(&schema, "stackMode", mode, &at)?,
            );
            if let Some(sp) = spacing {
                set.insert(
                    "stackSpacing",
                    Val::Float(require_finite_f32(*sp, "set_auto_layout spacing")?),
                );
            }
            if let Some(csp) = counter_spacing {
                set.insert(
                    "stackCounterSpacing",
                    Val::Float(require_finite_f32(*csp, "set_auto_layout counter_spacing")?),
                );
            }
            if let Some([top, right, bottom, left]) = padding {
                set.insert(
                    "stackVerticalPadding",
                    Val::Float(require_finite_f32(*top, "set_auto_layout padding[0]")?),
                );
                set.insert(
                    "stackPaddingRight",
                    Val::Float(require_finite_f32(*right, "set_auto_layout padding[1]")?),
                );
                set.insert(
                    "stackPaddingBottom",
                    Val::Float(require_finite_f32(*bottom, "set_auto_layout padding[2]")?),
                );
                set.insert(
                    "stackHorizontalPadding",
                    Val::Float(require_finite_f32(*left, "set_auto_layout padding[3]")?),
                );
            }
            fn map_sizing(s: &str) -> &str {
                match s {
                    "HUG" => "RESIZE_TO_FIT_WITH_IMPLICIT_SIZE",
                    other => other,
                }
            }
            if let Some(ps) = primary_sizing {
                set.insert(
                    "stackPrimarySizing",
                    intern_node_enum(&schema, "stackPrimarySizing", map_sizing(ps), &at)?,
                );
            }
            if let Some(cs) = counter_sizing {
                set.insert(
                    "stackCounterSizing",
                    intern_node_enum(&schema, "stackCounterSizing", map_sizing(cs), &at)?,
                );
            }
            if let Some(pa) = primary_align {
                set.insert(
                    "stackPrimaryAlignItems",
                    intern_node_enum(&schema, "stackPrimaryAlignItems", pa, &at)?,
                );
            }
            if let Some(ca) = counter_align {
                set.insert(
                    "stackCounterAlignItems",
                    intern_node_enum(&schema, "stackCounterAlignItems", ca, &at)?,
                );
            }
            if let Some(w) = wrap {
                set.insert(
                    "stackWrap",
                    Val::Enum(intern(if *w { "WRAP" } else { "NO_WRAP" })),
                );
            }
            txn.modify(*target, set, Vec::new());
            layout_touched.push(*target);
        }
        EditOp::SetAutoLayoutChild {
            target,
            primary_fill,
            counter_stretch,
            positioning,
            min_size,
            max_size,
        } => {
            let node = ensure_exists(doc, *target)?;
            if node.kind == "BOOLEAN_OPERATION"
                && (*primary_fill == Some(true) || *counter_stretch == Some(true))
            {
                return Err(FigError::malformed(format!(
                    "cannot set Auto-Layout fill or stretch on BOOLEAN_OPERATION {target}: derived boolean geometry cannot be resized"
                )));
            }
            let mut set = Obj::new();
            let mut unset = Vec::new();
            if let Some(pf) = primary_fill {
                set.insert(
                    "stackChildPrimaryGrow",
                    Val::Float(if *pf { 1.0 } else { 0.0 }),
                );
            }
            if let Some(cs) = counter_stretch {
                set.insert(
                    "stackChildAlignSelf",
                    Val::Enum(intern(if *cs { "STRETCH" } else { "AUTO" })),
                );
            }
            if let Some(pos) = positioning {
                let schema = doc.decoded_schema()?;
                let at = format!("node {target}");
                set.insert(
                    "stackPositioning",
                    intern_node_enum(&schema, "stackPositioning", pos, &at)?,
                );
            }
            if let Some([mw, mh]) = min_size {
                require_finite_f32(*mw, "set_auto_layout_child min_size[0]")?;
                require_finite_f32(*mh, "set_auto_layout_child min_size[1]")?;
                set.insert(
                    "minSize",
                    Val::Obj(Obj::from([(
                        "value",
                        Val::Obj(Obj::from([("x", Val::Float(*mw)), ("y", Val::Float(*mh))])),
                    )])),
                );
            }
            if let Some([xw, xh]) = max_size {
                require_finite_f32(*xw, "set_auto_layout_child max_size[0]")?;
                require_finite_f32(*xh, "set_auto_layout_child max_size[1]")?;
                set.insert(
                    "maxSize",
                    Val::Obj(Obj::from([(
                        "value",
                        Val::Obj(Obj::from([("x", Val::Float(*xw)), ("y", Val::Float(*xh))])),
                    )])),
                );
            } else if max_size.is_none() {
                let _ = &mut unset;
            }
            txn.modify(*target, set, unset);
            if let Some(p) = node.parent {
                layout_touched.push(p);
            }
        }
        EditOp::SetSolidFill { target, hex } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            match hex {
                Some(h) => set.insert("fillPaints", Val::Arr(vec![hex_paint(h)?])),
                None => set.insert("fillPaints", Val::Arr(Vec::new())),
            };
            unset_present(
                &node.data,
                &["styleIdForFill", "inheritFillStyleID"],
                &mut unset,
            );
            txn.modify(*target, set, unset);
        }
        EditOp::SetImageFill {
            target,
            sha1_hex,
            scale_mode,
            opacity,
        } => {
            let node = ensure_exists(doc, *target)?;
            let schema = doc.decoded_schema()?;
            let at = format!("node {target}");
            let scale_enum =
                intern_node_enum_path(&schema, &["fillPaints", "imageScaleMode"], scale_mode, &at)?;
            let paint = image_paint_with_scale_mode(sha1_hex, scale_enum, *opacity)?;
            let mut set = Obj::with_capacity(1);
            set.insert("fillPaints", Val::Arr(vec![paint]));
            let mut unset = Vec::new();
            unset_present(
                &node.data,
                &["styleIdForFill", "inheritFillStyleID"],
                &mut unset,
            );
            txn.modify(*target, set, unset);
        }
        EditOp::SetStroke {
            target,
            hex,
            weight,
            align,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            match hex {
                Some(h) => {
                    set.insert("strokePaints", Val::Arr(vec![hex_paint(h)?]));
                }
                None => {
                    set.insert("strokePaints", Val::Arr(Vec::new()));
                }
            }
            if let Some(w) = weight {
                let w = require_finite_f32(*w, "set_stroke weight")?;
                set.insert("strokeWeight", Val::Float(w.max(0.0)));
            }
            if let Some(a) = align {
                let schema = doc.decoded_schema()?;
                let at = format!("node {target}");
                set.insert(
                    "strokeAlign",
                    intern_node_enum(&schema, "strokeAlign", a, &at)?,
                );
            }
            unset_present(
                &node.data,
                &[
                    "styleIdForStrokeFill",
                    "inheritFillStyleIDForStroke",
                    "inheritStrokeStyleID",
                ],
                &mut unset,
            );
            if weight.is_some() || hex.is_none() {
                unset_present(
                    &node.data,
                    &[
                        "borderStrokeWeightsIndependent",
                        "borderTopWeight",
                        "borderRightWeight",
                        "borderBottomWeight",
                        "borderLeftWeight",
                    ],
                    &mut unset,
                );
            }
            txn.modify(*target, set, unset);
            if node.data.flag("bordersTakeSpace") == Some(true) {
                layout_touched.push(*target);
            }
        }
        EditOp::SetEffects {
            target,
            drop_shadow,
            blur_radius,
            background_blur,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut effs = Vec::new();
            if let Some(sh) = drop_shadow {
                for c in sh.rgba {
                    require_finite_f32(c, "set_effects drop_shadow rgba")?;
                }
                require_finite_f32(sh.offset[0], "set_effects drop_shadow offset[0]")?;
                require_finite_f32(sh.offset[1], "set_effects drop_shadow offset[1]")?;
                require_finite_f32(sh.radius, "set_effects drop_shadow radius")?;
                require_finite_f32(sh.spread, "set_effects drop_shadow spread")?;
                effs.push(shadow_effect(
                    sh.inner,
                    sh.rgba,
                    (sh.offset[0], sh.offset[1]),
                    sh.radius,
                    sh.spread,
                ));
            }
            if let Some(r) = blur_radius {
                let r = require_finite_f32(*r, "set_effects blur_radius")?;
                effs.push(blur_effect(*background_blur, r));
            }
            let mut set = Obj::with_capacity(1);
            set.insert("effects", Val::Arr(effs));
            let mut unset = Vec::new();
            unset_present(
                &node.data,
                &["styleIdForEffect", "inheritEffectStyleID"],
                &mut unset,
            );
            txn.modify(*target, set, unset);
        }
        EditOp::SetText { target, characters } => {
            set_text_plain(doc, txn, *target, characters)?;
            layout_touched.push(*target);
        }
        EditOp::SpliceText {
            target,
            utf16_start,
            utf16_delete_len,
            insert,
        } => {
            let node = ensure_exists(doc, *target)?;
            let base = node
                .data
                .get("textData")
                .cloned()
                .unwrap_or_else(|| Val::Obj(Obj::new()));
            let next = splice_text_utf16(&base, *utf16_start, *utf16_delete_len, insert)?;
            commit_text_data(doc, txn, *target, next)?;
            layout_touched.push(*target);
        }
        EditOp::StyleTextRange {
            target,
            utf16_start,
            utf16_end,
            style,
        } => {
            let node = ensure_exists(doc, *target)?;
            let schema = doc.decoded_schema()?;
            let base = node
                .data
                .get("textData")
                .cloned()
                .unwrap_or_else(|| Val::Obj(Obj::new()));
            let next = apply_text_range_style_with_schema(
                Some(&schema),
                &base,
                *utf16_start,
                *utf16_end,
                style,
            )?;
            commit_text_data(doc, txn, *target, next)?;
            layout_touched.push(*target);
        }
        EditOp::SetTypography {
            target,
            font_family,
            font_style,
            font_size,
            align_horizontal,
            auto_resize,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if font_family.is_some() || font_style.is_some() {
                let fam = font_family
                    .clone()
                    .or_else(|| {
                        node.data
                            .path(&["fontName", "family"])
                            .and_then(Val::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "Inter".into());
                let sty = font_style
                    .clone()
                    .or_else(|| {
                        node.data
                            .path(&["fontName", "style"])
                            .and_then(Val::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_else(|| "Regular".into());
                set.insert(
                    "fontName",
                    Val::Obj(Obj::from([
                        ("family", Val::Str(fam)),
                        ("style", Val::Str(sty)),
                        ("postscript", Val::Str(String::new())),
                    ])),
                );
            }
            if let Some(sz) = font_size {
                let sz = require_finite_f32(*sz, "set_typography font_size")?;
                set.insert("fontSize", Val::Float(sz.max(1.0)));
            }
            if align_horizontal.is_some() || auto_resize.is_some() {
                let schema = doc.decoded_schema()?;
                let at = format!("node {target}");
                if let Some(ah) = align_horizontal {
                    set.insert(
                        "textAlignHorizontal",
                        intern_node_enum(&schema, "textAlignHorizontal", ah, &at)?,
                    );
                }
                if let Some(ar) = auto_resize {
                    set.insert(
                        "textAutoResize",
                        intern_node_enum(&schema, "textAutoResize", ar, &at)?,
                    );
                }
            }
            if node.kind == "TEXT"
                && (font_family.is_some()
                    || font_style.is_some()
                    || font_size.is_some()
                    || auto_resize.is_some())
            {
                let mut preview = node.data.clone().into_owned();
                if let Val::Obj(ref mut map) = preview {
                    for (k, v) in set.iter() {
                        map.insert(k, v.clone());
                    }
                }
                let chars = preview
                    .path(&["textData", "characters"])
                    .and_then(Val::as_str)
                    .unwrap_or("");
                update_auto_resize_box(&preview, chars, &mut set);
            }
            txn.modify(*target, set, Vec::new());
            layout_touched.push(*target);
        }
        other => {
            compile_component_or_vector_op(doc, txn, alloc, other, created_ids, layout_touched)?
        }
    }
    Ok(())
}

fn unset_present(data: &Val, keys: &[&str], out: &mut Vec<String>) {
    for &key in keys {
        if data.get(key).is_some() {
            out.push(key.to_string());
        }
    }
}
