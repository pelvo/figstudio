//! Code Connect in design context: a layer whose component is mapped to code is written as
//! that code component (its snippet, or `<Name prop=… />`) instead of generated markup.
use figstudio_fig::{Document, Guid, Val, resolve};
use html_escape::encode_double_quoted_attribute as attr;
use std::collections::BTreeMap;

use crate::codeconnect_context::component_set;

/// A code component a Figma component is mapped to (the Code Connect sidecar's entry).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeComponent {
    /// The component's name in code, for example `Button`.
    pub name: String,
    /// Where it lives, for example `src/components/Button.tsx`.
    pub source: String,
    /// Code to use as written instead of `<Name … />`.
    pub snippet: Option<String>,
    /// Import lines; without them one is made from `source`.
    pub imports: Vec<String>,
}

/// Mappings by component (or component set) node ID.
pub type CodeComponents = BTreeMap<String, CodeComponent>;

/// The mapping of the component behind `data` (an instance, or a component), looked up on
/// the component, then its component set.
pub(super) fn mapping<'m>(
    doc: &Document,
    kind: &str,
    id: Option<Guid>,
    data: &Val,
    mappings: &'m CodeComponents,
) -> Option<(Guid, &'m CodeComponent)> {
    if mappings.is_empty() {
        return None;
    }
    let component = match kind {
        "INSTANCE" => resolve::symbol_id(data)?,
        "SYMBOL" => id?,
        _ => return None,
    };
    let set = component_set(doc, component).map(|s| s.id);
    [Some(component), set]
        .into_iter()
        .flatten()
        .find_map(|key| mappings.get(&key.to_string()).map(|m| (component, m)))
}

/// The import line(s) of a mapping.
pub(super) fn imports(m: &CodeComponent) -> Vec<String> {
    if !m.imports.is_empty() {
        return m.imports.clone();
    }
    // `src/components/Button.tsx` → `src/components/Button`.
    let path = m.source.split(['?', '#']).next().unwrap_or("");
    let module = path
        .rsplit_once('.')
        .filter(|(_, ext)| matches!(*ext, "ts" | "tsx" | "js" | "jsx" | "mjs" | "vue" | "svelte"))
        .map_or(path, |(stem, _)| stem);
    vec![format!("import {{ {} }} from \"{module}\"", m.name)]
}

/// JSX props of an instance: its variant values and component property values, by the
/// property names written in camelCase. Slots and instance swaps are left out.
pub(super) fn props(doc: &Document, component: Guid, data: &Val) -> Vec<String> {
    let Some(node) = doc.node(component) else {
        return Vec::new();
    };
    let set = component_set(doc, component);
    let defs = node
        .data
        .get("componentPropDefs")
        .or_else(|| set.and_then(|s| s.data.get("componentPropDefs")))
        .map_or(&[][..], Val::items);
    let name_of = |id: Option<Guid>| {
        defs.iter()
            .find(|d| d.get("id").and_then(Val::as_guid) == id && id.is_some())
            .and_then(|d| d.text("name"))
    };
    let mut out: Vec<(String, String)> = Vec::new();
    // Variant values come from the variant (component) the instance shows.
    for spec in node
        .data
        .get("variantPropSpecs")
        .map_or(&[][..], Val::items)
    {
        let (Some(name), Some(value)) = (
            name_of(spec.get("propDefId").and_then(Val::as_guid)),
            spec.text("value"),
        ) else {
            continue;
        };
        out.push((camel(name), format!("\"{}\"", attr(value))));
    }
    for a in data
        .get("componentPropAssignments")
        .map_or(&[][..], Val::items)
    {
        let Some(name) = name_of(a.get("defID").and_then(Val::as_guid)) else {
            continue;
        };
        let value = a.get("value").or_else(|| a.path(&["varValue", "value"]));
        let Some(value) = value else { continue };
        let written = if let Some(b) = value.get("boolValue").and_then(Val::as_bool) {
            if b {
                "{true}".to_owned()
            } else {
                "{false}".to_owned()
            }
        } else if let Some(text) = value
            .get("textValue")
            .and_then(|t| t.text("characters").or_else(|| t.as_str()))
        {
            format!("\"{}\"", attr(text))
        } else {
            continue;
        };
        let key = camel(name);
        out.retain(|(k, _)| *k != key);
        out.push((key, written));
    }
    out.into_iter().map(|(k, v)| format!("{k}={v}")).collect()
}

/// `Show Number` → `showNumber`, `size#12:3` → `size` (Figma suffixes property names).
fn camel(name: &str) -> String {
    let base = name.split('#').next().unwrap_or(name);
    let mut out = String::new();
    for (i, word) in base
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .enumerate()
    {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                out.extend(first.to_lowercase());
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    if out.is_empty() { "value".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_imports() {
        assert_eq!(camel("Show Number"), "showNumber");
        assert_eq!(camel("Label#12:3"), "label");
        let m = CodeComponent {
            name: "Button".into(),
            source: "src/components/Button.tsx".into(),
            ..Default::default()
        };
        assert_eq!(
            imports(&m),
            ["import { Button } from \"src/components/Button\""]
        );
    }
}
