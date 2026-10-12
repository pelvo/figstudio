//! Text layers: typography classes, style runs as spans, JSX-safe text.
use super::{Classes, Notes, num, paint, px};
use figstudio_fig::{
    Val,
    tokens::{Tokens, font_weight},
};

/// Typography classes of a text node, or of a style-run fragment (absent fields are skipped).
pub(super) fn classes(out: &mut Classes, data: &Val, tokens: &Tokens, notes: &mut Notes) {
    // An attached text style is the source of truth for the fields it sets.
    let styled = data
        .get("styleIdForText")
        .or_else(|| data.get("inheritTextStyleID"))
        .and_then(|r| tokens.style(r))
        .map(|s| &s.data);
    let field = |key: &str| styled.and_then(|s| s.get(key)).or_else(|| data.get(key));
    if let Some(family) = field("fontName").and_then(|f| f.text("family")) {
        out.push(format!(
            "font-['{}']",
            family.replace(['\'', ']', '[', '\\'], "").replace(' ', "_")
        ));
    }
    let style = field("fontName").and_then(|f| f.text("style"));
    // NodeChange has no numeric weight (only FontMetaData does); the style name carries it.
    if let Some(style) = style {
        out.push(format!("font-[{}]", font_weight(style)));
    }
    if style.is_some_and(|s| s.to_ascii_lowercase().contains("italic")) {
        out.push("italic");
    }
    if let Some(size) = field("fontSize").and_then(Val::as_f32) {
        out.push(format!("text-[{}]", px(f64::from(size))));
    }
    if let Some(leading) = measure(field("lineHeight"), true) {
        out.push(format!("leading-[{leading}]"));
    }
    let spacing = field("letterSpacing").filter(|v| v.num("value").is_some_and(|n| n != 0.0));
    if let Some(tracking) = measure(spacing, false) {
        out.push(format!("tracking-[{tracking}]"));
    }
    match field("textCase").and_then(Val::as_str) {
        Some("UPPER") => out.push("uppercase"),
        Some("LOWER") => out.push("lowercase"),
        Some("TITLE") => out.push("capitalize"),
        Some("SMALL_CAPS" | "SMALL_CAPS_FORCED") => out.push("[font-variant-caps:small-caps]"),
        _ => {}
    }
    match field("textDecoration").and_then(Val::as_str) {
        Some("UNDERLINE") => out.push("underline"),
        Some("STRIKETHROUGH") => out.push("line-through"),
        _ => {}
    }
    match data.text("textAlignHorizontal") {
        Some("CENTER") => out.push("text-center"),
        Some("RIGHT") => out.push("text-right"),
        Some("JUSTIFIED") => out.push("text-justify"),
        _ => {}
    }
    let fills = paint::paints(data, "fillPaints", tokens);
    if let Some(top) = fills.last() {
        match top.text("type") {
            Some("SOLID") => {
                if let Some(c) = paint::color(top, top.num("opacity").unwrap_or(1.0), tokens) {
                    out.push(format!("text-[color:{c}]"));
                }
            }
            _ => notes.add("non-solid text fills dropped"),
        }
    }
}

/// A Figma `Number` {value, units} as the renderer reads it. Line height: PIXELS is px,
/// PERCENT a percentage of the font size, RAW a unitless font-size multiplier (zero or less
/// is the font's natural height and skipped). Letter spacing: PERCENT is `em`/100, otherwise px.
fn measure(value: Option<&Val>, line_height: bool) -> Option<String> {
    let v = value?;
    let n = f64::from(v.num("value")?);
    let units = v.text("units").unwrap_or("RAW");
    if line_height {
        return (n > 0.0).then(|| match units {
            "PIXELS" => px(n),
            "PERCENT" => format!("{}%", num(n)),
            _ => num(n),
        });
    }
    Some(match units {
        "PERCENT" => format!("{}em", num(n / 100.0)),
        _ => px(n),
    })
}

/// Text split into runs of the same `styleID`. `characterStyleIDs` holds one id per UTF-16
/// unit (Figma strings are JavaScript strings); units past its end use the node style (0).
pub(super) fn runs(data: &Val) -> Vec<(u32, String)> {
    let text = data
        .path(&["textData", "characters"])
        .and_then(Val::as_str)
        .unwrap_or("");
    let ids = data
        .path(&["textData", "characterStyleIDs"])
        .map(Val::items)
        .unwrap_or_default();
    let mut runs: Vec<(u32, String)> = Vec::new();
    let mut unit = 0;
    for c in text.chars() {
        let id = ids.get(unit).and_then(Val::as_u32).unwrap_or(0);
        unit += c.len_utf16();
        match runs.last_mut() {
            Some((last, s)) if *last == id => s.push(c),
            _ => runs.push((id, c.to_string())),
        }
    }
    runs
}

/// The style-run fragment with `id` in `textData.styleOverrideTable`.
pub(super) fn fragment(data: &Val, id: u32) -> Option<&Val> {
    data.path(&["textData", "styleOverrideTable"])?
        .items()
        .iter()
        .find(|f| f.get("styleID").and_then(Val::as_u32) == Some(id))
}

/// Text as JSX children: HTML-escaped by `html-escape`, braces as entities (JSX expressions),
/// line breaks as `<br />`.
pub(super) fn jsx(text: &str) -> String {
    let escaped = html_escape::encode_text(text)
        .replace('{', "&#123;")
        .replace('}', "&#125;");
    escaped
        .split(['\n', '\u{2028}', '\u{2029}'])
        .collect::<Vec<_>>()
        .join("<br />")
}
