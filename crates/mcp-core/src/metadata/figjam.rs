//! FigJam attributes, text and counters added to the metadata XML by `get_figjam`.
//! Field names come from the Kiwi schema (`STICKY`, `CONNECTOR`, … are schema-only in the
//! public fixtures); a field that is absent is left out.
use super::node::capped;
use crate::variables::format::color;
use figstudio_fig::Val;
use std::collections::BTreeMap;

/// Kinds the renderer draws as a plain box with no text or shape, or not at all.
const NOT_DRAWN: [&str; 12] = [
    "STICKY",
    "SHAPE_WITH_TEXT",
    "CONNECTOR",
    "CODE_BLOCK",
    "TABLE",
    "TABLE_CELL",
    "WIDGET",
    "STAMP",
    "MEDIA",
    "WASHI_TAPE",
    "HIGHLIGHT",
    "SECTION_OVERLAY",
];
/// Kinds that get an optional screenshot (connectors are lines, cells are part of a table).
const PICTURED: [&str; 7] = [
    "STICKY",
    "SHAPE_WITH_TEXT",
    "SECTION",
    "TABLE",
    "CODE_BLOCK",
    "WIDGET",
    "STAMP",
];

/// What the walk saw: node ids to picture and a count of kinds the renderer does not draw.
#[derive(Debug, Default)]
pub(crate) struct Extras {
    /// Ids of nodes worth a screenshot, in document order.
    pub(crate) pictured: Vec<String>,
    undrawn: BTreeMap<String, usize>,
}

impl Extras {
    /// Records one node written to the XML.
    pub(crate) fn note(&mut self, id: &str, kind: &str) {
        if PICTURED.contains(&kind) {
            self.pictured.push(id.to_owned());
        }
        if NOT_DRAWN.contains(&kind) {
            *self.undrawn.entry(super::node::tag_name(kind)).or_default() += 1;
        }
    }

    /// The XML comment text naming FigJam kinds the renderer does not draw.
    pub(crate) fn approximations(&self) -> Option<String> {
        if self.undrawn.is_empty() {
            return None;
        }
        let kinds: Vec<String> = self
            .undrawn
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect();
        Some(format!(
            "approximations: FigJam nodes drawn as plain boxes or not at all ({})",
            kinds.join(", ")
        ))
    }
}

fn enum_name(data: &Val, key: &str) -> Option<String> {
    data.text(key).map(str::to_ascii_lowercase)
}

/// The first visible solid fill, as a hex colour.
fn fill_color(data: &Val) -> Option<String> {
    data.get("fillPaints")?
        .items()
        .iter()
        .filter(|p| p.flag("visible") != Some(false))
        .find(|p| matches!(p.text("type"), None | Some("SOLID")))
        .and_then(|p| color(p.get("color")?, p.num("opacity").unwrap_or(1.0)))
}

/// Extra attributes for FigJam kinds.
pub(super) fn attributes(kind: &str, data: &Val) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let mut add = |key: &'static str, value: Option<String>| {
        if let Some(value) = value {
            out.push((key, value));
        }
    };
    match kind {
        "STICKY" => add("color", fill_color(data)),
        "SHAPE_WITH_TEXT" => {
            add("shape", enum_name(data, "shapeWithTextType"));
            add("color", fill_color(data));
        }
        "CONNECTOR" => {
            let endpoint = |key: &str| data.get(key);
            let node = |e: Option<&Val>| e.and_then(|e| e.get("endpointNodeID")?.as_guid());
            let magnet = |e: Option<&Val>| e.and_then(|e| enum_name(e, "magnet"));
            let (start, end) = (endpoint("connectorStart"), endpoint("connectorEnd"));
            // An end attached to a node names it; a loose end carries its own point.
            let point = |e: Option<&Val>, axis: &str| {
                e.and_then(|e| e.path(&["position", axis])?.as_f32())
                    .map(|v| format!("{}", (f64::from(v) * 1000.0).round() / 1000.0))
            };
            add("start", node(start).map(|g| g.to_string()));
            if node(start).is_none() {
                add("startX", point(start, "x"));
                add("startY", point(start, "y"));
            }
            add("startMagnet", magnet(start).filter(|m| m != "none"));
            add("end", node(end).map(|g| g.to_string()));
            if node(end).is_none() {
                add("endX", point(end, "x"));
                add("endY", point(end, "y"));
            }
            add("endMagnet", magnet(end).filter(|m| m != "none"));
            add("lineStyle", enum_name(data, "connectorLineStyle"));
        }
        "SECTION" => add(
            "status",
            enum_name(data, "sectionStatus").filter(|s| s != "none"),
        ),
        "CODE_BLOCK" => add("language", enum_name(data, "codeBlockLanguage")),
        "WIDGET" => add(
            "widget",
            data.path(&["widgetMetadata", "widgetName"])
                .and_then(Val::as_str)
                .map(str::to_owned),
        ),
        _ => {}
    }
    out
}

/// The text shown inside a FigJam node; `None` for kinds without any.
pub(super) fn text(kind: &str, data: &Val) -> Option<String> {
    match kind {
        "STICKY" | "SHAPE_WITH_TEXT" | "TABLE_CELL" | "CONNECTOR" => data
            .path(&["textData", "characters"])
            .and_then(Val::as_str)
            .map(capped),
        "CODE_BLOCK" => data.text("sourceCode").map(capped),
        _ => None,
    }
}
