//! Formatting of stored style payloads; JSON stays in serde_json.
use figstudio_fig::{
    Val,
    tokens::{Style, Tokens, Value as TokenValue},
};
use serde_json::{Value, json};

pub(super) fn value(value: TokenValue) -> Option<Value> {
    match value {
        TokenValue::Color(v) => color(&v).map(Value::String),
        TokenValue::Float(v) if v.is_finite() => Some(json!(v)),
        TokenValue::Float(_) => None,
        TokenValue::String(v) => Some(json!(v)),
        TokenValue::Bool(v) => Some(json!(v)),
    }
}

fn color(v: &Val) -> Option<String> {
    let channels = [
        v.num("r")?,
        v.num("g")?,
        v.num("b")?,
        v.num("a").unwrap_or(1.0),
    ];
    if channels.iter().any(|n| !n.is_finite()) {
        return None;
    }
    let [r, g, b, a] = channels.map(|n| (n.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(if channels[3] < 1.0 {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}")
    })
}

pub(super) fn style(style: &Style, tokens: &Tokens) -> Option<Value> {
    match style.kind.as_str() {
        "FILL" | "STROKE" => {
            let paints: Option<Vec<_>> = style
                .paints
                .as_ref()?
                .items()
                .iter()
                .map(|p| paint(p, tokens))
                .collect();
            compact(paints?)
        }
        "TEXT" => {
            let d = &style.text;
            Some(json!(format!(
                "Font(family: {}, style: {}, size: {}, weight: {}, lineHeight: {}, letterSpacing: {})",
                json!(
                    d.path(&["fontName", "family"])
                        .and_then(Val::as_str)
                        .unwrap_or("")
                ),
                summary(d.path(&["fontName", "style"])),
                summary(d.get("fontSize")),
                d.num("fontWeight")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| font_weight(
                        d.path(&["fontName", "style"])
                            .and_then(Val::as_str)
                            .unwrap_or("")
                    )
                    .to_string()),
                summary(d.get("lineHeight")),
                summary(d.get("textTracking").or_else(|| d.get("letterSpacing")))
            )))
        }
        "EFFECT" => compact(
            style
                .effects
                .as_ref()?
                .items()
                .iter()
                .map(effect)
                .collect::<Option<Vec<_>>>()?,
        ),
        "GRID" => Some(raw(style.grids.as_ref()?)),
        _ => None,
    }
}

// Match the renderer's style-name interpretation without loading fonts.
fn font_weight(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace([' ', '-'], "");
    for (names, weight) in [
        (["thin", "hairline"], 100),
        (["extralight", "ultralight"], 200),
        (["semibold", "demibold"], 600),
        (["extrabold", "ultrabold"], 800),
        (["black", "heavy"], 900),
        (["bold", "bold"], 700),
        (["medium", "medium"], 500),
        (["light", "light"], 300),
    ] {
        if names.iter().any(|n| s.contains(n)) {
            return weight;
        }
    }
    400
}

fn compact(mut values: Vec<Value>) -> Option<Value> {
    match values.len() {
        0 => None,
        1 => values.pop(),
        _ => Some(Value::Array(values)),
    }
}

fn paint(paint: &Val, tokens: &Tokens) -> Option<Value> {
    let kind = paint.text("type")?;
    if kind == "SOLID" {
        let resolved = paint
            .get("colorVar")
            .and_then(|v| tokens.resolve(v, None))
            .and_then(value);
        return resolved.or_else(|| paint.get("color").and_then(color).map(Value::String));
    }
    if kind.starts_with("GRADIENT") {
        let stops: Option<Vec<_>> = paint
            .get("stops")?
            .items()
            .iter()
            .map(|stop| {
                let c = stop
                    .get("colorVar")
                    .and_then(|v| tokens.resolve(v, None))
                    .and_then(value)
                    .or_else(|| stop.get("color").and_then(color).map(Value::String))?;
                Some(json!({"position": raw(stop.get("position")?), "color": c}))
            })
            .collect();
        return Some(json!({"type": kind, "stops": stops?}));
    }
    None
}

fn effect(d: &Val) -> Option<Value> {
    let kind = d.text("type")?;
    let color = d
        .get("color")
        .and_then(color)
        .unwrap_or_else(|| "none".into());
    Some(json!(format!(
        "Effect(type: {kind}, color: {color}, offset: ({}, {}), radius: {}, spread: {})",
        summary(d.path(&["offset", "x"])),
        summary(d.path(&["offset", "y"])),
        summary(d.get("radius")),
        summary(d.get("spread"))
    )))
}

fn summary(value: Option<&Val>) -> String {
    value
        .map(raw)
        .map(|v| match v {
            Value::String(s) => s,
            _ => v.to_string(),
        })
        .unwrap_or_else(|| "null".into())
}

fn raw(value: &Val) -> Value {
    match value {
        Val::Bool(v) => json!(v),
        Val::Int(v) => json!(v),
        Val::Float(v) => json!(v),
        Val::Str(v) => json!(v),
        Val::Enum(v) => json!(v),
        Val::Arr(v) => Value::Array(v.iter().map(raw).collect()),
        Val::Obj(o) => Value::Object(o.iter().map(|(k, v)| (k.into(), raw(v))).collect()),
        Val::Bytes(_) => Value::Null,
    }
}
