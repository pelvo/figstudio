//! Classifies a property change into the row kinds the HTML report shows.

use crate::{Category, PropertyChange};

const FONT_KEYS: [&str; 5] = [
    "fontSize",
    "fontWeight",
    "textCase",
    "textDecoration",
    "fontName",
];

pub(crate) enum Kind<'a> {
    Colour {
        prefix: String,
        label: &'a str,
        ch: usize,
    },
    /// A paint's `opacity` or `visible`: its place ("Fill" or "Stroke") and that field.
    Paint {
        place: &'a str,
        field: &'a str,
    },
    Font,
    Hidden,
}

pub(crate) fn is_index(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn is_font_key(s: &str) -> bool {
    FONT_KEYS.contains(&s) || s.starts_with("lineHeight") || s.starts_with("letterSpacing")
}

/// Human field label: from the first font key on, else the last non-index segment.
pub(crate) fn label_of(field: &str) -> String {
    let segs: Vec<&str> = field.split('.').collect();
    match segs.iter().position(|s| is_font_key(s)) {
        Some(at) => segs[at..].join("."),
        None => segs
            .iter()
            .rev()
            .find(|s| !is_index(s))
            .unwrap_or(&field)
            .to_string(),
    }
}

pub(crate) fn classify(change: &PropertyChange) -> Kind<'static> {
    let segs: Vec<&str> = change.field.split('.').collect();
    let n = segs.len();
    let label = |s: &str| match s {
        "fillPaints" => Some("Fill"),
        "strokePaints" => Some("Stroke"),
        _ => None,
    };
    let channel = ["r", "g", "b", "a"].iter().position(|c| *c == segs[n - 1]);
    if n >= 4
        && segs[n - 2] == "color"
        && is_index(segs[n - 3])
        && let (Some(label), Some(ch)) = (label(segs[n - 4]), channel)
    {
        return Kind::Colour {
            prefix: segs[..n - 2].join("."),
            label,
            ch,
        };
    }
    if n >= 3
        && let Some(field) = ["opacity", "visible"]
            .into_iter()
            .find(|f| *f == segs[n - 1])
        && is_index(segs[n - 2])
        && let Some(place) = label(segs[n - 3])
    {
        return Kind::Paint { place, field };
    }
    if change.category == Category::Font || segs.iter().any(|s| is_font_key(s)) {
        return Kind::Font;
    }
    Kind::Hidden
}
