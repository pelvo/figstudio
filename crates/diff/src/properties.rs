use std::collections::{BTreeMap, BTreeSet};

use figstudio_fig::Val;

use crate::{Category, PropertyChange, RunChange, TextChange};

pub(crate) fn changes(old: Option<&Val>, new: Option<&Val>) -> Vec<PropertyChange> {
    let mut pending = vec![(String::new(), old, new)];
    let mut out = Vec::new();
    while let Some((path, old, new)) = pending.pop() {
        if old == new {
            continue;
        }
        match (old, new) {
            (Some(Val::Obj(_)), Some(Val::Obj(_)))
            | (Some(Val::Obj(_)), None)
            | (None, Some(Val::Obj(_))) => {
                let keys: BTreeSet<_> = [old, new]
                    .into_iter()
                    .flatten()
                    .filter_map(|v| match v {
                        Val::Obj(o) => Some(o),
                        _ => None,
                    })
                    .flat_map(|o| o.keys())
                    .collect();
                if !keys.is_empty() {
                    for key in keys {
                        pending.push((
                            join(&path, key),
                            old.and_then(|v| v.get(key)),
                            new.and_then(|v| v.get(key)),
                        ));
                    }
                    continue;
                }
            }
            (Some(Val::Arr(_)), Some(Val::Arr(_)))
            | (Some(Val::Arr(_)), None)
            | (None, Some(Val::Arr(_))) => {
                let a = old.map(Val::items).unwrap_or_default();
                let b = new.map(Val::items).unwrap_or_default();
                if !a.is_empty() || !b.is_empty() {
                    for i in 0..a.len().max(b.len()) {
                        pending.push((join(&path, &i.to_string()), a.get(i), b.get(i)));
                    }
                    continue;
                }
            }
            _ => {}
        }
        out.push(PropertyChange {
            category: category(&path),
            field: path,
            old: old.cloned(),
            new: new.cloned(),
        });
    }
    out.sort_by(|a, b| a.field.cmp(&b.field));
    out
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.into()
    } else {
        format!("{path}.{key}")
    }
}

fn category(path: &str) -> Category {
    const FIELDS: &[(&[&str], Category)] = &[
        (
            &[
                "font",
                "fontName",
                "fontFamily",
                "fontSize",
                "fontWeight",
                "fontStyle",
                "letterSpacing",
                "lineHeight",
                "textCase",
                "textDecoration",
            ],
            Category::Font,
        ),
        (&["fillPaints", "fills", "fillGeometry"], Category::Fill),
        (
            &[
                "strokePaints",
                "strokes",
                "strokeWeight",
                "strokeAlign",
                "strokeGeometry",
                "dashPattern",
            ],
            Category::Stroke,
        ),
        (&["effects", "effect"], Category::Effect),
        (
            &[
                "symbolData",
                "symbolID",
                "symbolOverrides",
                "derivedSymbolData",
                "componentProperties",
                "componentPropertyReferences",
                "componentPropAssignments",
                "overrides",
            ],
            Category::Component,
        ),
        (
            &[
                "size",
                "transform",
                "rotation",
                "cornerRadius",
                "rectangleCornerRadii",
                "vectorNetwork",
                "vectorData",
            ],
            Category::Geometry,
        ),
        (
            &[
                "parentIndex",
                "stackMode",
                "stackSpacing",
                "stackPadding",
                "stackHorizontalPadding",
                "stackVerticalPadding",
                "stackPaddingTop",
                "stackPaddingRight",
                "stackPaddingBottom",
                "stackPaddingLeft",
                "stackPrimarySizing",
                "stackCounterSizing",
                "stackChildPrimaryGrow",
                "stackChildAlignSelf",
                "stackChildPrimarySizing",
                "stackChildCounterSizing",
                "stackPrimaryAlignItems",
                "stackCounterAlignItems",
                "layoutMode",
                "constraints",
                "horizontalConstraint",
                "verticalConstraint",
            ],
            Category::Layout,
        ),
        (
            &[
                "textData",
                "characters",
                "styleOverrideTable",
                "characterStyleIDs",
                "textAlignHorizontal",
                "textAlignVertical",
                "paragraphSpacing",
                "textAutoResize",
            ],
            Category::Text,
        ),
    ];
    // Inner font fields take precedence over their textData container.
    for (fields, category) in FIELDS {
        if path.split('.').any(|part| fields.contains(&part)) {
            return *category;
        }
    }
    Category::Other
}

pub(crate) fn text_change(old: &Val, new: &Val, changes: &[PropertyChange]) -> Option<TextChange> {
    if !changes
        .iter()
        .any(|c| matches!(c.category, Category::Text | Category::Font))
    {
        return None;
    }
    let characters = |v: &Val| {
        v.path(&["textData", "characters"])
            .or_else(|| v.get("characters"))
            .and_then(Val::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let mut runs: BTreeMap<usize, Vec<PropertyChange>> = BTreeMap::new();
    for change in changes {
        let parts: Vec<_> = change.field.split('.').collect();
        if let Some(index) = parts.windows(2).find_map(|p| {
            (p[0] == "styleOverrideTable")
                .then(|| p[1].parse().ok())
                .flatten()
        }) {
            runs.entry(index).or_default().push(change.clone());
        }
    }
    Some(TextChange {
        old: characters(old),
        new: characters(new),
        runs: runs
            .into_iter()
            .map(|(index, changes)| RunChange { index, changes })
            .collect(),
    })
}
