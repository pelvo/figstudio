//! Motion and shader details of the selected layer for the Properties panel, worked out once
//! per selection: finding a layer's timeline or shader scans the document, and the panel is
//! drawn every frame.

use std::cell::RefCell;

use figstudio_fig::{Document, Guid, Node};
use serde_json::Value;

/// What the Properties panel shows beyond the node's own fields.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Details {
    /// The layer's `get_motion_context` entry, when it is animated.
    pub motion: Option<Value>,
    /// One entry per shader fill or effect on the layer (`shaders::on_node`).
    pub shaders: Vec<Value>,
}

/// The details of the last layer asked for, keyed by document and node.
#[derive(Debug, Default)]
pub struct Cache(RefCell<Option<(u64, Guid, Details)>>);

impl Cache {
    /// The details of `node`, computed on the first call for it.
    pub fn get(&self, doc: &Document, node: &Node) -> Details {
        let mut slot = self.0.borrow_mut();
        if let Some((instance, id, details)) = slot.as_ref()
            && *instance == doc.instance
            && *id == node.id
        {
            return details.clone();
        }
        let details = compute(doc, node);
        *slot = Some((doc.instance, node.id, details.clone()));
        details
    }
}

fn compute(doc: &Document, node: &Node) -> Details {
    use figstudio_mcp_core::{motion, shaders};
    Details {
        motion: motion::has_motion(&node.data)
            .then(|| motion::node_motion(doc, &node.id.to_string()))
            .flatten(),
        shaders: shaders::on_node(doc, node),
    }
}

/// A short form of a keyframe value: `0.5`, `12, -3` for a point, the JSON otherwise.
pub fn value_text(v: &Value) -> String {
    let num = trim;
    match v {
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), num),
        Value::Bool(b) => if *b { "on" } else { "off" }.into(),
        Value::String(s) => s.clone(),
        Value::Object(o) if o.contains_key("x") && o.contains_key("y") && o.len() == 2 => format!(
            "{}, {}",
            num(o["x"].as_f64().unwrap_or(0.0)),
            num(o["y"].as_f64().unwrap_or(0.0))
        ),
        Value::Object(o) if ["r", "g", "b"].iter().all(|k| o.contains_key(*k)) => {
            let c = |k: &str| (o[k].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}", c("r"), c("g"), c("b"))
        }
        // A circle point: centre, radius and angle.
        Value::Object(o) if o.contains_key("center") && o.contains_key("radius") => {
            let mut out = format!(
                "{} · r {}",
                value_text(&o["center"]),
                num(o["radius"].as_f64().unwrap_or(0.0))
            );
            if let Some(angle) = o.get("angle").and_then(Value::as_f64) {
                out.push_str(&format!(" · {}°", num(angle)));
            }
            out
        }
        // A colour point: the colour and where it sits.
        Value::Object(o) if o.contains_key("color") && o.contains_key("position") => format!(
            "{} at {}",
            value_text(&o["color"]),
            value_text(&o["position"])
        ),
        other => other.to_string(),
    }
}

/// The colour of a shader value, for a swatch: `Some` for `{r, g, b[, a]}`.
pub fn colour_of(v: &Value) -> Option<egui::Color32> {
    let o = v.as_object()?;
    let c = |k: &str| Some((o.get(k)?.as_f64()?.clamp(0.0, 1.0) * 255.0).round() as u8);
    Some(egui::Color32::from_rgb(c("r")?, c("g")?, c("b")?))
}

/// A keyframe value in the units the rest of Properties uses: opacity in percent, rotation in
/// degrees (clockwise positive, as stored), scale as a factor.
pub fn motion_value(field: &str, v: &Value) -> String {
    let n = v.as_f64();
    match (field, n) {
        ("OPACITY", Some(n)) => format!("{}%", trim(n)),
        // Clockwise positive, as stored (see `rotation_turn`).
        ("MOTION_ROTATION", Some(n)) => format!("{}°", trim(n.to_degrees())),
        (f, Some(n)) if f.starts_with("MOTION_SCALE") => format!("{}×", trim(n)),
        ("MOTION_SCALE_XY", None) => format!("{}×", value_text(v)),
        _ => value_text(v),
    }
}

/// A rotation track as Figma's Motion panel shows it: the amount and the direction,
/// `360° clockwise`. Motion rotation is stored clockwise positive (radians).
pub fn rotation_turn(first: &Value, last: &Value) -> Option<String> {
    let delta = (last.as_f64()? - first.as_f64()?).to_degrees();
    let direction = if delta >= 0.0 {
        "clockwise"
    } else {
        "counter-clockwise"
    };
    Some(format!("{}° {direction}", trim(delta.abs())))
}

/// A length of time: `900 ms`, `1.5 s`.
pub fn duration(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{} s", trim((ms / 100.0).round() / 10.0))
    } else {
        format!("{} ms", ms.round())
    }
}

/// An easing type (Figma's public motion names) as a person names it: `EASE_OUT` → `ease out`.
pub fn easing_label(kind: &str) -> String {
    match kind {
        "LINEAR" => "linear".into(),
        "HOLD" => "hold".into(),
        "EASE_IN" => "ease in".into(),
        "EASE_OUT" => "ease out".into(),
        "EASE_IN_AND_OUT" => "ease in and out".into(),
        "EASE_IN_BACK" => "ease in back".into(),
        "EASE_OUT_BACK" => "ease out back".into(),
        "EASE_IN_AND_OUT_BACK" => "ease in and out back".into(),
        "CUSTOM_CUBIC_BEZIER" => "custom curve".into(),
        "CUSTOM_SPRING" => "custom spring".into(),
        "GENTLE" | "QUICK" | "BOUNCY" | "SLOW" => format!("{} spring", kind.to_lowercase()),
        k if k.contains("SPRING") => "spring".into(),
        other => other.to_lowercase().replace('_', " "),
    }
}

/// How a timeline plays.
pub fn loop_label(mode: &str) -> &'static str {
    match mode {
        "loop" => "loops",
        "boomerang" => "back and forth",
        _ => "plays once",
    }
}

fn trim(n: f64) -> String {
    let r = (n * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

/// A field name as the panel shows it: `MOTION_SCALE_X` → `Scale X`.
pub fn field_label(field: &str) -> String {
    let words = field
        .strip_prefix("MOTION_")
        .unwrap_or(field)
        .to_lowercase()
        .replace('_', " ")
        .replace("translation", "position");
    let mut out = words
        .replace(" xy", "")
        .replace(" x", " X")
        .replace(" y", " Y");
    if let Some(first) = out.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn open(name: &str) -> Document {
        Document::open(format!(
            "{}/../../tests/fixtures/{name}.fig",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    #[test]
    fn an_animated_layer_has_its_tracks_and_a_plain_one_has_nothing() {
        let doc = open("motion");
        let cache = Cache::default();
        let animated = doc.node(Guid(1, 1106)).unwrap();
        let details = cache.get(&doc, animated);
        let motion = details.motion.expect("motion");
        assert_eq!(motion["timelineRootId"], "1:1105");
        assert_eq!(motion["keyframeBindings"].as_array().unwrap().len(), 3);
        let plain = doc.node(Guid(1, 1105)).unwrap();
        assert_eq!(cache.get(&doc, plain), Details::default());
    }

    #[test]
    fn a_shader_fill_lists_its_values_by_label_with_select_options_named() {
        let doc = open("shaders");
        let layer = doc.node(Guid(1, 14622)).unwrap();
        let details = Cache::default().get(&doc, layer);
        let [shader] = &details.shaders[..] else {
            panic!("{:?}", details.shaders)
        };
        assert_eq!(shader["field"], "fillPaints[1]");
        assert_eq!(shader["type"], "fill");
        let values = shader["values"].as_array().unwrap();
        assert_eq!(values[0], json!({"label": "Shape", "value": "Triangle"}));
        // Not set on the layer: the default shows.
        assert!(
            values
                .iter()
                .any(|v| v["label"] == "Ring count" && v["value"] == 20.0)
        );
        assert!(
            values
                .iter()
                .any(|v| v["label"] == "Offset" && v["value"] == 0.77)
        );
    }

    #[test]
    fn values_and_fields_read_as_in_the_panel() {
        assert_eq!(value_text(&json!(0.30000001)), "0.3");
        assert_eq!(value_text(&json!({"x": 0.0, "y": -20.0})), "0, -20");
        assert_eq!(
            value_text(&json!({"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0})),
            "#FF0000"
        );
        assert_eq!(field_label("MOTION_SCALE_X"), "Scale X");
        assert_eq!(field_label("MOTION_TRANSLATION_XY"), "Position");
        assert_eq!(field_label("MOTION_TRANSLATION_X"), "Position X");
        assert_eq!(field_label("OPACITY"), "Opacity");
        // The units Layout and Layer use.
        assert_eq!(motion_value("OPACITY", &json!(100.0)), "100%");
        assert_eq!(
            motion_value("MOTION_ROTATION", &json!(-std::f64::consts::TAU)),
            "-360°"
        );
        assert_eq!(motion_value("MOTION_SCALE_X", &json!(0.3)), "0.3×");
        assert_eq!(
            value_text(&json!({"angle": -64.3, "center": {"x": 50.0, "y": 50.0}, "radius": 144.0})),
            "50, 50 · r 144 · -64.3°"
        );
        assert_eq!(duration(901.444), "901 ms");
        assert_eq!(duration(3601.0), "3.6 s");
        assert_eq!(easing_label("EASE_OUT"), "ease out");
        assert_eq!(easing_label("QUICK"), "quick spring");
        // The playground's rotation preset: Figma's Motion panel says "Clockwise 360°".
        assert_eq!(
            rotation_turn(&json!(-std::f64::consts::TAU), &json!(0.0)).as_deref(),
            Some("360° clockwise")
        );
    }
}
