//! Formatting of stored style payloads; JSON stays in serde_json.
use figstudio_fig::{
    Val,
    tokens::{Style, StyleKind, Tokens, Value as TokenValue, font_weight},
};
use serde_json::{Value, json};

pub(super) fn value(value: TokenValue) -> Option<Value> {
    match value {
        TokenValue::Color(v) => color(&v, 1.0).map(Value::String),
        TokenValue::Float(v) if v.is_finite() => Some(number(v)),
        TokenValue::Float(_) => None,
        TokenValue::String(v) => Some(json!(v)),
        TokenValue::Bool(v) => Some(json!(v)),
    }
}

/// `#RRGGBB`, or `#RRGGBBAA` when not opaque, from a stored colour message.
pub(crate) fn color(v: &Val, opacity: f32) -> Option<String> {
    let channels = [
        v.num("r")?,
        v.num("g")?,
        v.num("b")?,
        v.num("a").unwrap_or(1.0) * opacity,
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
    match &style.kind {
        StyleKind::Fill | StyleKind::Stroke => {
            let paints: Option<Vec<_>> = style
                .paints
                .as_ref()?
                .items()
                .iter()
                .filter(|p| p.flag("visible") != Some(false))
                .map(|p| paint(p, tokens))
                .collect();
            compact(paints?)
        }
        StyleKind::Text => {
            let d = &style.data;
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
                    .map(|v| number(f64::from(v)).to_string())
                    .unwrap_or_else(|| font_weight(
                        d.path(&["fontName", "style"])
                            .and_then(Val::as_str)
                            .unwrap_or("")
                    )
                    .to_string()),
                measurement(d.get("lineHeight"), true),
                measurement(
                    d.get("textTracking").or_else(|| d.get("letterSpacing")),
                    false
                )
            )))
        }
        StyleKind::Effect => compact(
            style
                .effects
                .as_ref()?
                .items()
                .iter()
                .map(effect)
                .collect::<Option<Vec<_>>>()?,
        ),
        StyleKind::Grid => Some(raw(style.grids.as_ref()?)),
        _ => None,
    }
}

// PIXELS are bare px numbers; percentages retain their unit and RAW zero means auto.
fn measurement(value: Option<&Val>, line_height: bool) -> String {
    let Some(v) = value else { return "null".into() };
    let Some(n) = v.get("value") else {
        return summary(value);
    };
    match v.text("units").unwrap_or("RAW") {
        "PERCENT" => format!("{}%", summary(Some(n))),
        "RAW" if line_height && v.num("value") == Some(0.0) => "auto".into(),
        "RAW" | "PIXELS" => summary(Some(n)),
        unit => format!("{} {unit}", summary(Some(n))),
    }
}

fn number(v: f64) -> Value {
    let normalized = (v as f32).to_string().parse::<f64>().unwrap_or(v);
    if normalized.fract() == 0.0 && normalized >= i64::MIN as f64 && normalized < i64::MAX as f64 {
        json!(normalized as i64)
    } else {
        json!(normalized)
    }
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
    let opacity = paint.num("opacity").unwrap_or(1.0);
    if kind == "SOLID" {
        return paint_color(paint, tokens, opacity).map(Value::String);
    }
    if kind.starts_with("GRADIENT") {
        let stops: Option<Vec<_>> = paint
            .get("stops")?
            .items()
            .iter()
            .map(|stop| {
                let c = paint_color(stop, tokens, opacity)?;
                Some(json!({"position": raw(stop.get("position")?), "color": c}))
            })
            .collect();
        return Some(json!({"type": kind, "stops": stops?}));
    }
    None
}

fn paint_color(paint: &Val, tokens: &Tokens, opacity: f32) -> Option<String> {
    let resolved = paint.get("colorVar").and_then(|v| tokens.resolve(v, None));
    let c = match &resolved {
        Some(TokenValue::Color(c)) => c,
        _ => paint.get("color")?,
    };
    color(c, opacity)
}

fn effect(d: &Val) -> Option<Value> {
    let kind = d.text("type")?;
    let color = d
        .get("color")
        .and_then(|v| color(v, 1.0))
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
        Val::Float(v) => number(*v),
        Val::Str(v) => json!(v),
        Val::Enum(v) => json!(v),
        Val::Arr(v) => Value::Array(v.iter().map(raw).collect()),
        Val::Obj(o) => Value::Object(o.iter().map(|(k, v)| (k.into(), raw(v))).collect()),
        Val::Bytes(_) => Value::Null,
    }
}
