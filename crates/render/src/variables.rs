//! Resolve paint styles and paint-variable aliases once per render, without a per-paint
//! document scan.
use std::borrow::Cow;
use std::collections::HashMap;

use figstudio_fig::{Document, Guid, Val};

#[derive(Default)]
pub(crate) struct Variables {
    by_id: HashMap<Guid, Val>,
    by_asset: HashMap<(String, String), Val>,
    /// Paint style nodes by guid and by library asset (key, version); styles keep paints in
    /// `fillPaints`, for fill and stroke styles alike.
    styles: HashMap<Guid, Val>,
    styles_by_asset: HashMap<(String, String), Val>,
}

/// A node's style reference and the paint list that the style replaces.
pub(crate) const STYLED_PAINTS: [(&str, &str); 2] = [
    ("styleIdForFill", "fillPaints"),
    ("styleIdForStrokeFill", "strokePaints"),
];

impl Variables {
    pub fn new(doc: &Document, remaining: &mut usize) -> Self {
        let mut out = Self::default();
        for node in &doc.nodes {
            if *remaining == 0 {
                break;
            }
            *remaining -= 1;
            if node.data.get("styleType").is_some()
                && let Some(paints) = node.data.get("fillPaints")
            {
                out.styles.insert(node.id, paints.clone());
                if let (Some(key), Some(version)) =
                    (node.data.text("key"), node.data.text("version"))
                {
                    out.styles_by_asset
                        .insert((key.into(), version.into()), paints.clone());
                }
            }
            if node.kind != "VARIABLE" {
                continue;
            }
            // shortcut: use the first stored mode; add inherited mode selection when needed.
            let Some(value) = node
                .data
                .path(&["variableDataValues", "entries"])
                .and_then(|v| v.items().first())
                .and_then(|e| e.get("variableData"))
            else {
                continue;
            };
            out.by_id.insert(node.id, value.clone());
            if let (Some(key), Some(version)) = (node.data.text("key"), node.data.text("version")) {
                out.by_asset
                    .insert((key.into(), version.into()), value.clone());
            }
        }
        out
    }

    pub fn color(&self, data: &Val, depth: usize) -> Option<Val> {
        if depth >= 24 {
            return None;
        }
        let value = data.get("value")?;
        if let Some(color) = value.get("colorValue") {
            return Some(color.clone());
        }
        let alias = value.get("alias")?;
        let target = if let Some(id) = alias.get("guid").and_then(Val::as_guid) {
            self.by_id.get(&id)
        } else {
            let asset = alias.get("assetRef")?;
            self.by_asset
                .get(&(asset.text("key")?.into(), asset.text("version")?.into()))
        }?;
        self.color(target, depth + 1)
    }

    /// Paints of the style a `styleIdFor*` reference points at; `None` for the detached
    /// sentinel or a style missing from the file, which keep the node's stored paints.
    fn style(&self, reference: &Val) -> Option<&Val> {
        if let Some(id) = reference.get("guid").and_then(Val::as_guid) {
            return self.styles.get(&id);
        }
        let asset = reference.get("assetRef")?;
        self.styles_by_asset
            .get(&(asset.text("key")?.into(), asset.text("version")?.into()))
    }

    pub fn paints<'a>(&self, data: &'a Val) -> (Cow<'a, Val>, usize) {
        // An attached style is the source of truth; the node's own paints are a cache that
        // can be stale after the style changed.
        let styled: Vec<(&str, &Val)> = STYLED_PAINTS
            .iter()
            .filter_map(|(style, key)| Some((*key, self.style(data.get(style)?)?)))
            .filter(|(key, paints)| data.get(key) != Some(*paints))
            .collect();
        if styled.is_empty() {
            return self.variable_paints(data);
        }
        let mut data = data.clone();
        for (key, paints) in styled {
            data.set(key, paints.clone());
        }
        let (resolved, unresolved) = self.variable_paints(&data);
        (Cow::Owned(resolved.into_owned()), unresolved)
    }

    fn variable_paints<'a>(&self, data: &'a Val) -> (Cow<'a, Val>, usize) {
        let has_variables = ["fillPaints", "strokePaints"].iter().any(|key| {
            data.get(key)
                .map(Val::items)
                .unwrap_or_default()
                .iter()
                .any(|p| p.get("colorVar").is_some())
        });
        let has_styles = data
            .path(&["vectorData", "styleOverrideTable"])
            .is_some_and(|v| !v.items().is_empty());
        if !has_variables && !has_styles {
            return (Cow::Borrowed(data), 0);
        }
        let mut data = data.clone();
        // Resolve vector style paints too, because those replace the node's paints.
        let mut style_unresolved = 0;
        if let Some(Val::Arr(styles)) = data
            .get_mut("vectorData")
            .and_then(|v| v.get_mut("styleOverrideTable"))
        {
            for style in styles {
                let (resolved, count) = self.paints(style);
                *style = resolved.into_owned();
                style_unresolved += count;
            }
        }
        let mut unresolved = 0;
        for key in ["fillPaints", "strokePaints"] {
            let Some(paints) = data.get_mut(key) else {
                continue;
            };
            let Val::Arr(paints) = paints else {
                continue;
            };
            for paint in paints {
                let Some(variable) = paint.get("colorVar") else {
                    continue;
                };
                if let Some(color) = self.color(variable, 0) {
                    paint.set("color", color);
                } else {
                    unresolved += 1;
                }
            }
        }
        (Cow::Owned(data), unresolved + style_unresolved)
    }
}
