//! Paint, stroke and effect [`EditOp`]s: fill and stroke lists, stroke style, effect lists.

use crate::edit::Txn;
use crate::edit::specs::{EffectSpec, PaintSlot, PaintSpec};
use crate::{Document, FigError, Guid, Obj, Val, intern};

use super::EditOp;
use super::compile_tree::ensure_exists;

pub(super) fn compile_style_op(
    doc: &Document,
    txn: &mut Txn,
    op: &EditOp,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::SetPaints {
            target,
            slot,
            paints,
        } => {
            let list = paints
                .iter()
                .map(PaintSpec::to_val)
                .collect::<Result<Vec<_>, _>>()?;
            edit_paints(doc, txn, *target, *slot, |l| {
                *l = list;
                Ok(())
            })?;
        }
        EditOp::AddPaint {
            target,
            slot,
            paint,
            index,
        } => {
            let v = paint.to_val()?;
            edit_paints(doc, txn, *target, *slot, |l| {
                let at = index.unwrap_or(l.len());
                check_insert(at, l.len())?;
                l.insert(at, v);
                Ok(())
            })?;
        }
        EditOp::UpdatePaint {
            target,
            slot,
            index,
            patch,
        } => edit_paints(doc, txn, *target, *slot, |l| patch.apply(item(l, *index)?))?,
        EditOp::RemovePaint {
            target,
            slot,
            index,
        } => edit_paints(doc, txn, *target, *slot, |l| {
            item(l, *index)?;
            l.remove(*index);
            Ok(())
        })?,
        EditOp::MovePaint {
            target,
            slot,
            from,
            to,
        } => edit_paints(doc, txn, *target, *slot, |l| move_item(l, *from, *to))?,
        EditOp::SetStrokeStyle {
            target,
            weight,
            align,
            cap,
            join,
            miter_limit,
            dashes,
            sides,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            let mut unset = Vec::new();
            let num = |v: f64, what: &str| {
                if v.is_finite() && v >= 0.0 {
                    Ok(Val::Float(v))
                } else {
                    Err(FigError::malformed(format!(
                        "{what} must be a finite, non-negative number"
                    )))
                }
            };
            if let Some(w) = weight {
                set.insert("strokeWeight", num(*w, "weight")?);
            }
            if let Some(a) = align {
                set.insert("strokeAlign", Val::Enum(intern(&a.to_ascii_uppercase())));
            }
            if let Some(c) = cap {
                set.insert("strokeCap", Val::Enum(intern(&c.to_ascii_uppercase())));
            }
            if let Some(j) = join {
                set.insert("strokeJoin", Val::Enum(intern(&j.to_ascii_uppercase())));
            }
            if let Some(m) = miter_limit {
                set.insert("miterLimit", num(*m, "miter limit")?);
            }
            if let Some(d) = dashes {
                let list = d
                    .iter()
                    .map(|v| num(*v, "dash"))
                    .collect::<Result<Vec<_>, _>>()?;
                if list.is_empty() {
                    unset.push("dashPattern".to_string());
                } else {
                    set.insert("dashPattern", Val::Arr(list));
                }
            }
            match sides {
                Some([t, r, b, l]) => {
                    set.insert("borderStrokeWeightsIndependent", Val::Bool(true));
                    set.insert("borderTopWeight", num(*t, "top weight")?);
                    set.insert("borderRightWeight", num(*r, "right weight")?);
                    set.insert("borderBottomWeight", num(*b, "bottom weight")?);
                    set.insert("borderLeftWeight", num(*l, "left weight")?);
                }
                None if weight.is_some()
                    && node.data.flag("borderStrokeWeightsIndependent") == Some(true) =>
                {
                    // One weight for all sides again.
                    set.insert("borderStrokeWeightsIndependent", Val::Bool(false));
                }
                None => {}
            }
            // The stored outline is for the old style; the renderer restrokes without it.
            if node.data.get("strokeGeometry").is_some() {
                unset.push("strokeGeometry".to_string());
            }
            txn.modify(*target, set, unset);
            layout_touched.push(*target);
        }
        EditOp::AddEffect {
            target,
            effect,
            index,
        } => {
            let v = effect.to_val()?;
            edit_effects(doc, txn, *target, |l| {
                let at = index.unwrap_or(l.len());
                check_insert(at, l.len())?;
                l.insert(at, v);
                Ok(())
            })?;
        }
        EditOp::UpdateEffect {
            target,
            index,
            patch,
        } => edit_effects(doc, txn, *target, |l| patch.apply(item(l, *index)?))?,
        EditOp::RemoveEffect { target, index } => edit_effects(doc, txn, *target, |l| {
            item(l, *index)?;
            l.remove(*index);
            Ok(())
        })?,
        EditOp::MoveEffect { target, from, to } => {
            edit_effects(doc, txn, *target, |l| move_item(l, *from, *to))?;
        }
        _ => return Err(FigError::malformed("not a style op")),
    }
    Ok(())
}

/// Replaces the effect list with `effects` (the list form of `set_effects`).
pub(super) fn set_effect_list(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    effects: &[EffectSpec],
) -> Result<(), FigError> {
    let list = effects
        .iter()
        .map(EffectSpec::to_val)
        .collect::<Result<Vec<_>, _>>()?;
    edit_effects(doc, txn, target, |l| {
        *l = list;
        Ok(())
    })
}

fn item(list: &mut [Val], index: usize) -> Result<&mut Val, FigError> {
    let len = list.len();
    list.get_mut(index).ok_or_else(|| {
        FigError::malformed(format!("index {index} is out of range (list has {len})"))
    })
}

fn check_insert(at: usize, len: usize) -> Result<(), FigError> {
    if at > len {
        return Err(FigError::malformed(format!(
            "index {at} is out of range (list has {len})"
        )));
    }
    Ok(())
}

fn move_item(list: &mut Vec<Val>, from: usize, to: usize) -> Result<(), FigError> {
    item(list, from)?;
    if to >= list.len() {
        return Err(FigError::malformed(format!(
            "index {to} is out of range (list has {})",
            list.len()
        )));
    }
    let v = list.remove(from);
    list.insert(to, v);
    Ok(())
}

/// Reads `field`'s list, lets `f` change it and writes it back, detaching `style_field`.
fn edit_list(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    field: &str,
    style_field: &str,
    f: impl FnOnce(&mut Vec<Val>) -> Result<(), FigError>,
) -> Result<(), FigError> {
    let node = ensure_exists(doc, target)?;
    let mut list: Vec<Val> = node
        .data
        .get(field)
        .map(|v| v.items().iter().map(|i| i.clone().into_owned()).collect())
        .unwrap_or_default();
    f(&mut list)?;
    let mut set = Obj::new();
    set.insert(field, Val::Arr(list));
    // An edited list no longer matches the style it came from, as in Figma.
    let unset = if node.data.get(style_field).is_some() {
        vec![style_field.to_string()]
    } else {
        Vec::new()
    };
    txn.modify(target, set, unset);
    Ok(())
}

fn edit_paints(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    slot: PaintSlot,
    f: impl FnOnce(&mut Vec<Val>) -> Result<(), FigError>,
) -> Result<(), FigError> {
    edit_list(doc, txn, target, slot.field(), slot.style_field(), f)
}

fn edit_effects(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    f: impl FnOnce(&mut Vec<Val>) -> Result<(), FigError>,
) -> Result<(), FigError> {
    edit_list(doc, txn, target, "effects", "styleIdForEffect", f)
}
