//! HTML rows for shared-component updates: a SYMBOL version bump, and the cached resolved
//! content (`derived*`) of its instances. Only meaningful fields are summarised.

use crate::html_changes::plain;
use crate::{NodeDiff, PropertyChange};

const MAX_ITEMS: usize = 4;

pub(crate) fn is_derived(change: &PropertyChange) -> bool {
    change.field.split('.').any(|s| s.starts_with("derived"))
}

pub(crate) fn is_version(change: &PropertyChange) -> bool {
    matches!(
        change.field.as_str(),
        "sharedSymbolVersion" | "userFacingVersion"
    )
}

fn after<'a>(segs: &'a [&'a str], key: &str) -> Option<&'a [&'a str]> {
    segs.iter().position(|s| *s == key).map(|i| &segs[i + 1..])
}

fn pair(c: &PropertyChange) -> String {
    format!("{} → {}", plain(c.old.as_ref()), plain(c.new.as_ref()))
}

/// One `component content updated: …` row, or `None` when only ignored fields changed.
pub(crate) fn content_row(changes: &[&PropertyChange]) -> Option<String> {
    let mut items: Vec<String> = Vec::new();
    let (mut text, mut size_changed) = (false, false);
    let (mut size_x, mut size_y) = (None, None);
    let mut fonts: Vec<(String, String, String)> = Vec::new();
    for c in changes {
        let segs: Vec<&str> = c.field.split('.').collect();
        if let Some([_, rest @ ..]) = after(&segs, "glyphs") {
            if c.old.is_none() || c.new.is_none() {
                text = true;
            } else if rest == ["fontSize"] {
                items.push(format!("fontSize {}", pair(c)));
            }
        } else if let Some([_, "key", key @ ("family" | "fontStyle" | "fontWeight")]) =
            after(&segs, "fontMetaData")
        {
            let name = key.trim_start_matches("font").to_lowercase();
            fonts.push((name, plain(c.old.as_ref()), plain(c.new.as_ref())));
        } else if segs.contains(&"logicalIndexToCharacterOffsetMap") {
            text = true;
        } else if segs.contains(&"layoutSize") {
            size_changed = true;
        } else if let Some(rest) =
            after(&segs, "derivedSymbolData").filter(|_| segs.contains(&"size"))
        {
            match rest {
                ["0", "size", "x"] => size_x = Some(*c),
                ["0", "size", "y"] => size_y = Some(*c),
                _ => size_changed = true,
            }
        }
    }
    // Index shifts show up as `X → (absent)` plus `(absent) → X`: not a real change.
    let absent = "(absent)";
    let shifted = |n: &str, v: &str| {
        let has = |o: &str, w: &str| fonts.iter().any(|f| f.0 == n && f.1 == o && f.2 == w);
        has(v, absent) && has(absent, v)
    };
    let real: Vec<String> = fonts
        .iter()
        .filter(|(n, o, w)| {
            o != w && !(o == absent && shifted(n, w)) && !(w == absent && shifted(n, o))
        })
        .map(|(n, o, w)| format!("{n} {o} → {w}"))
        .collect();
    items.extend(real);
    if let (Some(x), Some(y)) = (size_x, size_y) {
        let (old, new) = (
            format!("{}×{}", plain(x.old.as_ref()), plain(y.old.as_ref())),
            format!("{}×{}", plain(x.new.as_ref()), plain(y.new.as_ref())),
        );
        items.push(format!("size {old} → {new}"));
    } else if size_x.is_some() || size_y.is_some() || size_changed {
        items.push("size changed".into());
    }
    if text {
        items.push("text changed".into());
    }
    items.sort();
    items.dedup();
    if items.is_empty() {
        return None;
    }
    let more = items.len() > MAX_ITEMS;
    items.truncate(MAX_ITEMS);
    let tail = if more { ", …" } else { "" };
    Some(format!(
        "component content updated: {}{tail}",
        items.join(", ")
    ))
}

/// An INSTANCE whose only changes are ignored derived fields: its master changed elsewhere.
pub(crate) fn inherited_row(node: &NodeDiff) -> bool {
    node.kind == "INSTANCE"
        && node.moved.is_none()
        && node.text.is_none()
        && !node.changes.is_empty()
        && node.changes.iter().all(is_derived)
}
