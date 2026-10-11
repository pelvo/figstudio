//! Resolve paint styles and paint-variable aliases from an index built once per document.
use figstudio_fig::{
    Document, Val,
    tokens::{Tokens, Value},
};
use std::borrow::Cow;

#[derive(Default)]
pub(crate) struct Variables {
    tokens: Tokens,
    /// Some value in the document mentions a style reference or a paint variable; when none
    /// does, [`Variables::paints`] has nothing to resolve or count and returns at once.
    used: bool,
}

/// Fields whose presence anywhere (nodes or instance overrides) needs [`Variables::paints`].
const PAINT_REFERENCES: [&str; 4] = [
    "colorVar",
    "styleOverrideTable",
    "styleIdForFill",
    "styleIdForStrokeFill",
];

/// True when `value` or anything nested in it has one of [`PAINT_REFERENCES`].
fn mentions_paint_reference(value: &Val) -> bool {
    let mut stack = vec![value];
    while let Some(v) = stack.pop() {
        match v {
            Val::Obj(o) => {
                if o.keys().any(|k| PAINT_REFERENCES.contains(&k)) {
                    return true;
                }
                stack.extend(o.values());
            }
            Val::Arr(items) => stack.extend(items),
            _ => {}
        }
    }
    false
}

pub(crate) use figstudio_fig::resolve::STYLED_PAINTS;

impl Variables {
    pub fn new(doc: &Document) -> Self {
        Self {
            used: doc.nodes.iter().any(|n| mentions_paint_reference(&n.data)),
            tokens: Tokens::new(doc),
        }
    }

    pub fn color(&self, data: &Val, depth: usize) -> Option<Val> {
        // shortcut: preserve the first stored mode; inherited mode selection is separate.
        match self.tokens.resolve_at(data, None, depth) {
            Some(Value::Color(color)) => Some(color),
            _ => None,
        }
    }

    /// Attached styles supply paints; detached or missing styles keep stored paints.
    fn style(&self, reference: &Val) -> Option<&Val> {
        self.tokens.style(reference)?.paints.as_ref()
    }

    pub fn paints<'a>(&self, data: &'a Val) -> (Cow<'a, Val>, usize) {
        if !self.used {
            return (Cow::Borrowed(data), 0);
        }
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
