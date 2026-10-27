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
    let num = |n: f64| {
        let r = (n * 100.0).round() / 100.0;
        if r.fract() == 0.0 {
            format!("{r:.0}")
        } else {
            format!("{r}")
        }
    };
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
        other => other.to_string(),
    }
}

/// A field name as the panel shows it: `MOTION_SCALE_X` → `Scale X`.
pub fn field_label(field: &str) -> String {
    let words = field
        .strip_prefix("MOTION_")
        .unwrap_or(field)
        .replace('_', " ");
    let mut out = words.to_lowercase();
    if let Some(first) = out.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    out.replace(" xy", " XY")
        .replace(" x", " X")
        .replace(" y", " Y")
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
        assert_eq!(field_label("MOTION_TRANSLATION_XY"), "Translation XY");
        assert_eq!(field_label("OPACITY"), "Opacity");
    }
}
