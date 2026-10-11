//! One metadata element: attributes, text and escaping.
use super::{MAX_TEXT_CHARS, figjam};
use figstudio_fig::Val;
use quick_xml::{
    Writer,
    events::{BytesStart, BytesText, Event},
};

/// Writes one element for a node of type `kind` with (resolved) `data`, shown as `id`.
/// `figjam` adds the FigJam attributes and text. Returns whether the element was left open
/// (children or text follow).
pub(super) fn write_node(
    writer: &mut Writer<Vec<u8>>,
    id: &str,
    kind: &str,
    data: &Val,
    children: bool,
    figjam: bool,
) -> std::io::Result<bool> {
    let mut attrs = attributes(id, kind, data);
    if figjam {
        attrs.extend(figjam::attributes(kind, data));
    }
    let mut tag = BytesStart::new(tag_name(kind));
    for (key, value) in &attrs {
        tag.push_attribute((*key, value.as_str()));
    }
    let text = if figjam {
        figjam::text(kind, data).unwrap_or_else(|| node_text(kind, data))
    } else {
        node_text(kind, data)
    };
    let needs_end = children || !text.is_empty();
    if needs_end {
        writer.write_event(Event::Start(tag))?;
        if !text.is_empty() {
            writer.write_event(Event::Text(BytesText::new(&text)))?;
        }
    } else {
        writer.write_event(Event::Empty(tag))?;
    }
    Ok(needs_end)
}

/// The XML element name for a node type: `ROUNDED_RECTANGLE` becomes `rounded-rectangle`.
pub(super) fn tag_name(kind: &str) -> String {
    kind.to_ascii_lowercase().replace('_', "-")
}

fn attributes(id: &str, kind: &str, data: &Val) -> Vec<(&'static str, String)> {
    let num = |field: &str, key: &str, default: f64| {
        data.get(field)
            .and_then(|value| value.num(key))
            .map_or(default, f64::from)
    };
    let name = clean_xml(data.text("name").unwrap_or(""));
    let mut attrs = vec![("id", id.to_owned()), ("name", name)];
    if !matches!(kind, "CANVAS" | "DOCUMENT") {
        attrs.extend([
            ("x", number(num("transform", "m02", 0.0))),
            ("y", number(num("transform", "m12", 0.0))),
        ]);
        if data.get("size").is_some() {
            attrs.extend([
                ("width", number(num("size", "x", 0.0))),
                ("height", number(num("size", "y", 0.0))),
            ]);
        }
    }
    if data.flag("visible") == Some(false) {
        attrs.push(("hidden", "true".into()));
    }
    let rotation = number(
        (-num("transform", "m10", 0.0))
            .atan2(num("transform", "m00", 1.0))
            .to_degrees(),
    );
    if rotation != "0" {
        attrs.push(("rotation", rotation));
    }
    // A swapped instance shows the component it was swapped to.
    if kind == "INSTANCE"
        && let Some(id) = figstudio_fig::resolve::symbol_id(data)
    {
        attrs.push(("componentId", id.to_string()));
    }
    attrs
}

pub(super) fn clean_xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(*c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}'))
        .collect()
}

fn node_text(kind: &str, data: &Val) -> String {
    if kind != "TEXT" {
        return String::new();
    }
    capped(
        data.path(&["textData", "characters"])
            .and_then(Val::as_str)
            .unwrap_or(""),
    )
}

/// `value` without control characters, cut to [`MAX_TEXT_CHARS`] plus an ellipsis.
pub(super) fn capped(value: &str) -> String {
    let clean = clean_xml(value);
    let mut chars = clean.chars();
    let mut text: String = chars.by_ref().take(MAX_TEXT_CHARS).collect();
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".into();
    }
    let value = (value * 1000.0).round() / 1000.0;
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}
