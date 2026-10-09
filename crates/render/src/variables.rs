//! Resolve paint-variable aliases once per render, without a per-paint document scan.
use std::borrow::Cow;
use std::collections::HashMap;

use figstudio_fig::{Document, Guid, Val};

#[derive(Default)]
pub(crate) struct Variables {
    by_id: HashMap<Guid, Val>,
    by_asset: HashMap<(String, String), Val>,
}

impl Variables {
    pub fn new(doc: &Document, remaining: &mut usize) -> Self {
        let mut out = Self::default();
        for node in &doc.nodes {
            if *remaining == 0 {
                break;
            }
            *remaining -= 1;
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

    pub fn paints<'a>(&self, data: &'a Val) -> (Cow<'a, Val>, usize) {
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
