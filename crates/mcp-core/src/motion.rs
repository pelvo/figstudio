//! `get_motion_context`: keyframe animation of a node (or its subtree) with CSS and motion.dev
//! code.
//!
//! How Figma stores motion (read from public files, not documented by Figma):
//! - An animated layer binds a field (`OPACITY`, `MOTION_SCALE_X`, `WIDTH`, …) in its
//!   `parameterConsumptionMap` to a `KEYFRAME` expression: the field's base value and a
//!   keyframe track (`KEYFRAME_TRACK` node) per animation, manual or from a preset.
//! - A track's `KEYFRAME` children hold `timelinePosition` (µs), `keyframeValue` and
//!   `easingData`. Its `keyframeOperation` says how a value combines with the base value:
//!   `SET` (or none) replaces it, `SCALE` multiplies it, `OFFSET` adds to it.
//! - A preset track's parent is an `ANIMATION_PRESET_INSTANCE`, whose `timelineOffset` (µs)
//!   delays the whole track.
//! - Easing on a keyframe shapes the segment that ends at it (the first keyframe's is unused).
//! - `MOTION_ROTATION` is in radians, clockwise positive: Figma's Motion panel shows the
//!   playground's -2π → 0 track as "Clockwise 360°".
//! - A timeline (`timelineDefinitions` entry, or `timelineDuration` on a frame) gives the
//!   duration and the playback style (`ONCE`, `LOOP`, `BOOMERANG`). An instance shifts its
//!   component's timeline by `timelineAssignments[].value.offsetUs`.
use crate::{
    CoreError,
    node_id::{NodeId, parse_node_id},
    output::Output,
    shaders::tidy,
};
use figstudio_fig::{
    Document, Guid, Val,
    resolve::{self, Resolved},
};
use serde_json::{Map, Value, json};
use std::{collections::HashMap, fmt::Write as _, sync::Arc};

/// Most animated nodes one recursive call returns (Figma's own cap).
pub const MAX_NODES: usize = 500;
/// Most nodes a recursive call looks at.
const MAX_VISITED: usize = 100_000;
/// Samples of a spring in a CSS `linear()` easing.
const SPRING_SAMPLES: usize = 32;
/// Stand-in for spring presets whose parameters the file does not store.
const DEFAULT_SPRING: Spring = Spring {
    stiffness: 100.0,
    damping: 15.0,
    mass: 1.0,
};

const EASING_NOTE: &str = "Easing on a keyframe shapes the segment that ends at it.";
const ROTATION_NOTE: &str = "rotate: Figma stores motion rotation in radians, clockwise positive \
(checked against Figma's Motion panel), the same direction as CSS and motion.dev degrees.";
const TIMING_NOTE: &str = "Snippets run every track over the whole timeline (delays and holds \
are keyframes), so tracks of one timeline stay in step and loops line up.";

/// One animated value of a node.
struct Binding {
    field: String,
    operation: &'static str,
    preset: Option<String>,
    /// Timeline the track names, if any.
    timeline: Option<Guid>,
    /// Keyframes with the delay applied and the operation resolved, sorted by time.
    keyframes: Vec<Keyframe>,
    /// The motion path between translation keyframes is curved; snippets go straight.
    curved: bool,
}

struct Keyframe {
    time_us: f64,
    value: Value,
    easing: Easing,
}

#[derive(Clone, Debug, PartialEq)]
enum Easing {
    Linear,
    Hold,
    Cubic(&'static str, [f64; 4]),
    Spring(&'static str, Option<Spring>, Option<[f64; 4]>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Spring {
    stiffness: f64,
    damping: f64,
    mass: f64,
}

/// A node with motion: what to report and where its timeline comes from.
struct Animated {
    id: String,
    name: String,
    kind: String,
    bindings: Vec<Binding>,
    root: String,
    duration_us: f64,
    loop_mode: &'static str,
    /// The layer's static rotation in CSS degrees (clockwise); Figma's `rotate` is absolute.
    base_rotation: f64,
    /// The layer's pivot (`transformOrigin`) as a CSS value; `None` for the centre default.
    origin: Option<String>,
    /// The layer's static path trim (start, end), fractions of the path.
    trim: (f64, f64),
    /// The layer's per-character/word/line delay (`textElementDelayBehaviour`), when enabled.
    text: Option<Value>,
    /// Vector-like layers become `motion.svg`, others `motion.div`.
    vector: bool,
}

/// Accumulates what the snippets could not express exactly.
#[derive(Default)]
struct Notes {
    springs_sampled: usize,
    springs_assumed: usize,
    springs_motion: usize,
    curved_paths: usize,
    unconverted: Vec<String>,
    /// Fields in the motion.dev snippet that the CSS snippet cannot express.
    css_missing: Vec<String>,
    merged_axes: usize,
    /// Nodes whose snippets repeat an earlier node's.
    duplicates: usize,
}

/// Whether a node's data binds any field to keyframes (cheap: no track lookups).
pub fn has_motion(data: &Val) -> bool {
    data.path(&["parameterConsumptionMap", "entries"])
        .map_or(&[][..], Val::items)
        .iter()
        .any(|e| {
            e.path(&[
                "variableData",
                "value",
                "expressionValue",
                "expressionFunction",
            ])
            .and_then(Val::as_str)
                == Some("KEYFRAME")
        })
}

/// One node's own animation as its `get_motion_context` entry (timeline, duration, loop mode,
/// keyframe bindings, snippets); `None` when it has none.
pub fn node_motion(doc: &Document, node_id: &str) -> Option<Value> {
    let out = build(doc, node_id, false, usize::MAX).ok()?;
    out["nodes"].get(0).cloned()
}

/// The CSS snippet of one node's animation (its own keyframes, not its descendants'); `None`
/// when it has none.
pub fn node_css(doc: &Document, node_id: &str) -> Option<String> {
    let out = build(doc, node_id, false, usize::MAX).ok()?;
    out["nodes"][0]["codeSnippets"]["css"]
        .as_str()
        .map(str::to_owned)
}

/// `get_motion_context` for `node_id` (node, instance path or link). `recursive` adds every
/// animated descendant, instances included, at most [`MAX_NODES`]; the reply stops before
/// `max_bytes` of JSON.
pub fn motion_context(doc: &Document, node_id: &str, recursive: bool, max_bytes: usize) -> Output {
    match build(doc, node_id, recursive, max_bytes) {
        Ok(out) => Output::structured(out),
        Err(error) => Output::fault(error.to_string()),
    }
}

fn build(
    doc: &Document,
    node_id: &str,
    recursive: bool,
    max_bytes: usize,
) -> Result<Value, CoreError> {
    let timelines = Timelines::new(doc);
    let mut found: Vec<Animated> = Vec::new();
    let mut visited = 0usize;
    let mut capped = false;
    let add = |found: &mut Vec<Animated>, a: Option<Animated>| found.extend(a);
    match parse_node_id(node_id)? {
        NodeId::Node(id) => {
            let Some(node) = doc.node(id) else {
                return Err(CoreError::input(format!(
                    "node {id} does not exist in this file"
                )));
            };
            if node.kind == "DOCUMENT" {
                return Err(CoreError::input("pick a page or a layer, not the document"));
            }
            // Built on the first instance only: it scans every node.
            let mut definitions = None;
            let mut stack = vec![id];
            while let Some(id) = stack.pop() {
                visited += 1;
                if visited > MAX_VISITED {
                    capped = true;
                    break;
                }
                let Some(node) = doc.node(id) else { continue };
                add(
                    &mut found,
                    animated(doc, &timelines, &id.to_string(), id, &node.data, None),
                );
                if !recursive {
                    break;
                }
                if node.kind == "INSTANCE"
                    && let Ok(walk) = resolve::walk_instance(
                        doc,
                        definitions.get_or_insert_with(|| Arc::new(resolve::definitions(doc))),
                        id,
                        None,
                        MAX_VISITED,
                    )
                {
                    capped |= walk.truncated;
                    for r in &walk.nodes {
                        add(&mut found, resolved(doc, &timelines, r, &walk.nodes));
                    }
                }
                stack.extend(doc.children(id).iter().rev());
            }
        }
        NodeId::InstancePath(path) => {
            let definitions = Arc::new(resolve::definitions(doc));
            let walk = resolve::walk_path(
                doc,
                &definitions,
                &path,
                if recursive { None } else { Some(0) },
                MAX_VISITED,
            )
            .map_err(|e| CoreError::resolve(&path, e))?;
            capped |= walk.truncated;
            for r in &walk.nodes {
                add(&mut found, resolved(doc, &timelines, r, &walk.nodes));
            }
        }
    }

    let total = found.len();
    if found.len() > MAX_NODES {
        capped = true;
        found.truncate(MAX_NODES);
    }
    let mut notes = Notes::default();
    let mut nodes = Vec::new();
    let mut bytes = 512;
    let mut cut = false;
    let mut seen: HashMap<String, String> = HashMap::new();
    for a in &found {
        let mut entry = node_entry(a, &mut notes);
        // Recursive replies replace a snippet identical to an earlier node's with a comment
        // naming that node, as Figma's do.
        if let Some(motion) = entry["codeSnippets"]["motion"].as_str().map(str::to_owned) {
            match seen.get(&motion) {
                Some(first) => {
                    notes.duplicates += 1;
                    entry["codeSnippets"] = json!({
                        "css": format!("/* Same motion as {first} */"),
                        "motion": format!("// Same motion as {first}"),
                    });
                }
                None => {
                    seen.insert(motion, a.id.clone());
                }
            }
        }
        let size = entry.to_string().len();
        if bytes + size > max_bytes && !nodes.is_empty() {
            cut = true;
            break;
        }
        bytes += size;
        nodes.push((a, entry));
    }
    let mut cohorts: Vec<(String, f64, &str, Vec<String>)> = Vec::new();
    for (a, _) in &nodes {
        match cohorts.iter_mut().find(|c| c.0 == a.root) {
            Some(c) => {
                c.1 = c.1.max(a.duration_us);
                c.3.push(a.id.clone());
            }
            None => cohorts.push((
                a.root.clone(),
                a.duration_us,
                a.loop_mode,
                vec![a.id.clone()],
            )),
        }
    }
    let mut text_notes = vec![json!(EASING_NOTE), json!(TIMING_NOTE)];
    if nodes
        .iter()
        .any(|(a, _)| a.bindings.iter().any(|b| b.field == "MOTION_ROTATION"))
    {
        text_notes.push(json!(ROTATION_NOTE));
    }
    for line in notes.lines() {
        text_notes.push(json!(line));
    }
    if nodes.is_empty() {
        text_notes.push(json!(if recursive {
            "No keyframe animation under this node."
        } else {
            "No keyframe animation on this node; call again with recursive: true for its descendants."
        }));
    }
    if cut {
        text_notes.push(json!(format!(
            "Output stopped at about {max_bytes} bytes (--max-output-bytes); call again with a child nodeId."
        )));
    }
    Ok(json!({
        "nodeId": node_id,
        "recursive": recursive,
        "nodes": nodes.into_iter().map(|(_, e)| e).collect::<Vec<_>>(),
        "timelineCohorts": cohorts.into_iter().map(|(root, d, mode, members)| json!({
            "rootNodeId": root,
            "durationMs": ms(d),
            "loopMode": mode,
            "memberNodeIds": members,
        })).collect::<Vec<_>>(),
        "total": total,
        "truncated": capped || cut,
        "notes": text_notes,
    }))
}

impl Notes {
    fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.springs_sampled > 0 {
            out.push(format!(
                "approximated: {} spring segments sampled into CSS linear() over the segment's duration",
                self.springs_sampled
            ));
        }
        if self.springs_assumed > 0 {
            out.push(format!(
                "approximated: {} spring preset segments; the file does not store their parameters, so stiffness 100, damping 15, mass 1 stand in",
                self.springs_assumed
            ));
        }
        if self.springs_motion > 0 {
            out.push(format!(
                "approximated: {} spring segments use a cubic-bezier in the motion.dev snippet",
                self.springs_motion
            ));
        }
        if self.curved_paths > 0 {
            out.push(format!(
                "approximated: {} curved motion paths drawn as straight moves",
                self.curved_paths
            ));
        }
        if self.merged_axes > 0 {
            out.push(format!(
                "approximated: {} x/y pairs with different keyframe times merged for CSS (missing values interpolated linearly)",
                self.merged_axes
            ));
        }
        if !self.css_missing.is_empty() {
            out.push(format!(
                "not in the CSS snippet (motion.dev snippet and keyframeBindings only): {}",
                self.css_missing.join(", ")
            ));
        }
        if self.duplicates > 0 {
            out.push(format!(
                "{} snippets repeat an earlier node's and are replaced by a comment naming it",
                self.duplicates
            ));
        }
        if !self.unconverted.is_empty() {
            out.push(format!(
                "not in the snippets (keyframeBindings only): {}",
                self.unconverted.join(", ")
            ));
        }
        out
    }
}

/// A document node's motion; `component` (the node the data comes from) gives the ancestors.
fn animated(
    doc: &Document,
    timelines: &Timelines,
    id: &str,
    component: Guid,
    data: &Val,
    instance: Option<(&str, f64)>,
) -> Option<Animated> {
    let node = doc.node(component)?;
    let delay = instance.map_or(0.0, |(_, offset)| offset);
    let bindings = bindings(doc, data, delay);
    if bindings.is_empty() {
        return None;
    }
    let named = bindings.iter().find_map(|b| b.timeline);
    let (root, mut duration, loop_mode) = timelines.of(doc, component, named);
    // Inside an instance, the component's timeline plays in the instance.
    let root = match instance {
        Some((top, _)) => top.to_owned(),
        None => root,
    };
    for b in &bindings {
        if let Some(last) = b.keyframes.last() {
            duration = duration.max(last.time_us);
        }
    }
    let transform = data.get("transform");
    let m = |k: &str| transform.and_then(|t| t.num(k)).map_or(0.0, f64::from);
    let base_rotation = if transform.is_some() {
        m("m10")
            .atan2(m("m00").max(f64::MIN_POSITIVE).copysign(m("m00")))
            .to_degrees()
    } else {
        0.0
    };
    let trim = data.path(&["pathTrim", "value"]).map_or((0.0, 1.0), |v| {
        (
            v.num("x").map_or(0.0, f64::from),
            v.num("y").map_or(1.0, f64::from),
        )
    });
    let text = data
        .get("textElementDelayBehaviour")
        .filter(|t| t.flag("enabled") == Some(true))
        .map(|t| {
            tidy(json!({
                "grouping": t.text("grouping").unwrap_or("CHARACTER"),
                "delayMs": t.num("delayMs").map_or(0.0, f64::from),
                "order": t.text("order").unwrap_or("LEFT_RIGHT"),
                "clipContent": t.flag("clipContent").unwrap_or(false),
                "transformOriginMode": t.text("transformOriginMode"),
            }))
        });
    Some(Animated {
        id: id.to_owned(),
        name: data.text("name").unwrap_or(&node.name).to_owned(),
        kind: node.kind.clone(),
        vector: matches!(
            node.kind.as_str(),
            "VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "LINE" | "REGULAR_POLYGON"
        ),
        bindings,
        root,
        duration_us: duration,
        loop_mode,
        base_rotation: if base_rotation.abs() < 1e-6 {
            0.0
        } else {
            base_rotation
        },
        origin: origin(data),
        trim,
        text,
    })
}

/// A node inside an instance, delayed by the timeline offsets of the instances around it.
fn resolved(
    doc: &Document,
    timelines: &Timelines,
    r: &Resolved,
    walk: &[Resolved],
) -> Option<Animated> {
    let top = r.path.instance();
    let keys = r.path.keys();
    // The outer instance, then every nested instance on the path.
    let mut offset = doc.node(top).map_or(0.0, |n| instance_offset(&n.data));
    for end in 1..keys.len() {
        let prefix = &keys[..end];
        if let Some(inner) = walk.iter().find(|w| w.path.keys() == prefix) {
            offset += instance_offset(&inner.data);
        }
    }
    animated(
        doc,
        timelines,
        &r.path.to_string(),
        r.node,
        &r.data,
        Some((&top.to_string(), offset)),
    )
}

/// The timeline offset an instance gives its component's animation (µs).
fn instance_offset(data: &Val) -> f64 {
    data.path(&["timelineAssignments", "entries"])
        .map_or(&[][..], Val::items)
        .iter()
        .filter(|e| e.path(&["value", "disabled"]).and_then(Val::as_bool) != Some(true))
        .find_map(|e| e.path(&["value", "offsetUs"]).and_then(number))
        .unwrap_or(0.0)
}

/// Every keyframe binding of `data`, delayed by `delay` µs.
fn bindings(doc: &Document, data: &Val, delay: f64) -> Vec<Binding> {
    let mut out = Vec::new();
    for entry in data
        .path(&["parameterConsumptionMap", "entries"])
        .map_or(&[][..], Val::items)
    {
        let Some(expression) = entry.path(&["variableData", "value", "expressionValue"]) else {
            continue;
        };
        if expression.text("expressionFunction") != Some("KEYFRAME") {
            continue;
        }
        let Some(field) = entry.text("variableField") else {
            continue;
        };
        let arguments = expression
            .get("expressionArguments")
            .map_or(&[][..], Val::items);
        let base = arguments.first().and_then(|a| a.get("value"));
        for argument in arguments.iter().skip(1) {
            for parameter in argument
                .path(&["value", "keyframeTrackParameterValue", "parameters"])
                .map_or(&[][..], Val::items)
            {
                let value = parameter.get("value");
                let link = value.and_then(|v| v.get("manual").or_else(|| v.get("animationPreset")));
                let Some(link) = link else { continue };
                let Some(track) = link
                    .path(&["keyframeTrackId", "guid"])
                    .and_then(Val::as_guid)
                else {
                    continue;
                };
                if let Some(b) = binding(doc, field, base, track, link, delay) {
                    out.push(b);
                }
            }
        }
    }
    out
}

fn binding(
    doc: &Document,
    field: &str,
    base: Option<&Val>,
    track: Guid,
    link: &Val,
    delay: f64,
) -> Option<Binding> {
    let node = doc.node(track)?;
    let operation = match node.data.text("keyframeOperation") {
        Some("SCALE") => "SCALE",
        Some("OFFSET") => "OFFSET",
        _ => "SET",
    };
    let preset_node = node
        .parent
        .and_then(|p| doc.node(p))
        .filter(|p| p.kind == "ANIMATION_PRESET_INSTANCE");
    let preset_delay = preset_node
        .and_then(|p| p.data.get("timelineOffset"))
        .and_then(number)
        .unwrap_or(0.0);
    let preset = preset_node.map(|p| {
        p.name
            .strip_prefix("motion.preset_name.")
            .unwrap_or(&p.name)
            .to_owned()
    });
    let mut curved = false;
    let mut keyframes: Vec<Keyframe> = doc
        .children(track)
        .iter()
        .filter_map(|k| doc.node(*k))
        .filter(|k| k.kind == "KEYFRAME" && k.data.flag("visible") != Some(false))
        .filter_map(|k| {
            let raw = k.data.path(&["keyframeValue", "value"])?;
            if let Some(h) = k.data.get("motionPathBezierHandles")
                && ["p1x", "p1y", "p2x", "p2y"]
                    .iter()
                    .any(|c| h.num(c).is_some_and(|v| v != 0.0))
            {
                curved = true;
            }
            Some(Keyframe {
                time_us: delay
                    + preset_delay
                    + k.data
                        .get("timelinePosition")
                        .and_then(number)
                        .unwrap_or(0.0),
                value: combine(operation, base.map(value_of), value_of(raw)),
                easing: easing(k.data.get("easingData")),
            })
        })
        .collect();
    if keyframes.is_empty() {
        return None;
    }
    keyframes.sort_by(|a, b| a.time_us.total_cmp(&b.time_us));
    Some(Binding {
        field: field.to_owned(),
        operation,
        preset,
        timeline: link.get("timelineDefId").and_then(Val::as_guid),
        keyframes,
        curved,
    })
}

/// A keyframe value as JSON: a number, `{x, y}`, or the stored object for other kinds.
fn value_of(value: &Val) -> Value {
    if let Some(f) = value.get("floatValue").and_then(number) {
        return json!(f);
    }
    if let Some(v) = value.get("vectorValue") {
        return json!({"x": v.num("x").map_or(0.0, f64::from), "y": v.num("y").map_or(0.0, f64::from)});
    }
    if let Some(b) = value.get("boolValue").and_then(Val::as_bool) {
        return json!(b);
    }
    match value {
        Val::Obj(fields) if fields.len() == 1 => fields.values().next().map_or(Value::Null, |v| {
            serde_json::to_value(v).unwrap_or(Value::Null)
        }),
        other => serde_json::to_value(other).unwrap_or(Value::Null),
    }
}

/// `value` combined with the field's base value by the track's operation.
fn combine(operation: &str, base: Option<Value>, value: Value) -> Value {
    let op = |a: f64, b: f64| if operation == "SCALE" { a * b } else { a + b };
    match (operation, base, &value) {
        ("SET", _, _) | (_, None, _) => value,
        (_, Some(Value::Number(b)), Value::Number(v)) => {
            json!(op(b.as_f64().unwrap_or(0.0), v.as_f64().unwrap_or(0.0)))
        }
        (_, Some(Value::Object(b)), Value::Object(v)) => {
            let axis =
                |m: &Map<String, Value>, k: &str| m.get(k).and_then(Value::as_f64).unwrap_or(0.0);
            json!({"x": op(axis(&b, "x"), axis(v, "x")), "y": op(axis(&b, "y"), axis(v, "y"))})
        }
        _ => value,
    }
}

fn easing(data: Option<&Val>) -> Easing {
    let Some(data) = data else {
        return Easing::Linear;
    };
    let stored = data.path(&["easingValue", "bezierEasing"]).and_then(|b| {
        Some([
            f64::from(b.num("p1x")?),
            f64::from(b.num("p1y")?),
            f64::from(b.num("p2x")?),
            f64::from(b.num("p2y")?),
        ])
    });
    let spring = data.path(&["easingValue", "springEasing"]).and_then(|s| {
        Some(Spring {
            stiffness: f64::from(s.num("stiffness")?),
            damping: f64::from(s.num("damping")?),
            mass: f64::from(s.num("mass").unwrap_or(1.0)),
        })
    });
    match data.text("easingType").unwrap_or("LINEAR") {
        "LINEAR" => Easing::Linear,
        "HOLD" => Easing::Hold,
        "CUSTOM_CUBIC" => Easing::Cubic("CUSTOM_CUBIC", stored.unwrap_or(STANDARD_EASE)),
        kind if kind.contains("SPRING") => Easing::Spring(intern_kind(kind), spring, stored),
        kind => match named_cubic(kind) {
            Some((_, _, curve)) => Easing::Cubic(intern_kind(kind), stored.unwrap_or(curve)),
            None => Easing::Cubic("CUSTOM_CUBIC", stored.unwrap_or(STANDARD_EASE)),
        },
    }
}

/// CSS's `ease`, used when a custom curve stores no handles.
const STANDARD_EASE: [f64; 4] = [0.25, 0.1, 0.25, 1.0];

/// The stored name as a `'static` string (the easing types are a fixed set).
fn intern_kind(kind: &str) -> &'static str {
    const KINDS: [&str; 16] = [
        "IN_CUBIC",
        "OUT_CUBIC",
        "INOUT_CUBIC",
        "LINEAR",
        "IN_BACK_CUBIC",
        "OUT_BACK_CUBIC",
        "INOUT_BACK_CUBIC",
        "CUSTOM_CUBIC",
        "SPRING",
        "GENTLE_SPRING",
        "CUSTOM_SPRING",
        "SPRING_PRESET_ONE",
        "SPRING_PRESET_TWO",
        "SPRING_PRESET_THREE",
        "HOLD",
        "EASE_IN",
    ];
    KINDS
        .iter()
        .find(|k| **k == kind)
        .copied()
        .unwrap_or("CUSTOM_CUBIC")
}

/// A stored named curve: Figma's public easing type, motion.dev's name, and the cubic-bezier
/// (CSS's `ease-in`/`ease-out`/`ease-in-out` and motion.dev's `back` curves).
fn named_cubic(kind: &str) -> Option<(&'static str, &'static str, [f64; 4])> {
    Some(match kind {
        "IN_CUBIC" | "EASE_IN" => ("EASE_IN", "easeIn", [0.42, 0.0, 1.0, 1.0]),
        "OUT_CUBIC" => ("EASE_OUT", "easeOut", [0.0, 0.0, 0.58, 1.0]),
        "INOUT_CUBIC" => ("EASE_IN_AND_OUT", "easeInOut", [0.42, 0.0, 0.58, 1.0]),
        "IN_BACK_CUBIC" => ("EASE_IN_BACK", "backIn", [0.31, 0.01, 0.66, -0.59]),
        "OUT_BACK_CUBIC" => ("EASE_OUT_BACK", "backOut", [0.33, 1.53, 0.69, 0.99]),
        "INOUT_BACK_CUBIC" => ("EASE_IN_AND_OUT_BACK", "backInOut", [0.68, -0.6, 0.32, 1.6]),
        _ => return None,
    })
}

/// Spring presets: Figma's public easing type and its bounce. The file stores only the
/// preset's name, not its parameters.
fn spring_preset(kind: &str) -> Option<(&'static str, f64)> {
    SPRING_PRESETS
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map(|(_, public, bounce)| (*public, *bounce))
}

/// Stored preset name, Figma's public name, bounce (read from Figma's Motion panel).
const SPRING_PRESETS: [(&str, &str, f64); 0] = [];

/// Figma's normalised spring `bounce` (0 to 1) of physical spring parameters: one minus the
/// damping ratio, as motion.dev defines bounce.
fn bounce(s: Spring) -> f64 {
    let zeta = s.damping / (2.0 * (s.stiffness.max(1e-9) * s.mass.max(1e-9)).sqrt());
    (1.0 - zeta).clamp(0.0, 1.0)
}

/// The physical spring for a bounce, at motion.dev's default stiffness and mass (for the CSS
/// sample of a preset that stores no parameters).
fn spring_of_bounce(bounce: f64) -> Spring {
    let (stiffness, mass) = (100.0, 1.0);
    Spring {
        stiffness,
        damping: 2.0 * (1.0 - bounce.clamp(0.0, 1.0)) * (stiffness * mass).sqrt(),
        mass,
    }
}

/// The layer's pivot (`transformOrigin`) as a CSS value, `None` when unset. Each axis is a
/// raw fraction of the size (Figma's docs: 3.098 is 309.8%), a percentage, or pixels.
fn origin(data: &Val) -> Option<String> {
    let o = data.get("transformOrigin")?;
    let axis = |k: &str| -> Option<String> {
        let v = o.get(k)?;
        let value = f64::from(v.num("value")?);
        Some(match v.text("units").unwrap_or("RAW") {
            "PIXELS" => format!("{}px", n(value)),
            "PERCENT" => format!("{}%", n(value)),
            _ => format!("{}%", n(value * 100.0)),
        })
    };
    let (x, y) = (axis("x")?, axis("y")?);
    (x != "50%" || y != "50%").then(|| format!("{x} {y}"))
}

/// Path trim tracks, drawn as a stroke dash on `motion.path`.
fn is_trim(field: &str) -> bool {
    matches!(field, "PATH_TRIM_START" | "PATH_TRIM_END")
}

impl Easing {
    /// The easing as Figma's public motion easing object (`type`, and
    /// `easingFunctionCubicBezier` or `easingFunctionSpring` where they apply).
    fn json(&self) -> Value {
        let bezier = |b: &[f64; 4]| tidy(json!({"x1": b[0], "y1": b[1], "x2": b[2], "y2": b[3]}));
        match self {
            Easing::Linear => json!({"type": "LINEAR"}),
            Easing::Hold => json!({"type": "HOLD"}),
            Easing::Cubic(kind, b) => match named_cubic(kind) {
                Some((public, _, _)) => json!({"type": public}),
                None => json!({
                    "type": "CUSTOM_CUBIC_BEZIER",
                    "easingFunctionCubicBezier": bezier(b),
                }),
            },
            Easing::Spring(kind, spring, _) => match (spring_preset(kind), spring) {
                (Some((public, _)), _) => json!({"type": public}),
                (None, Some(s)) => json!({
                    "type": "CUSTOM_SPRING",
                    "easingFunctionSpring": tidy(json!({"bounce": bounce(*s)})),
                }),
                (None, None) => json!({"type": kind}),
            },
        }
    }

    /// The spring's bounce, when this is a spring with a known one.
    fn bounce(&self) -> Option<f64> {
        match self {
            Easing::Spring(kind, spring, _) => spring_preset(kind)
                .map(|(_, b)| b)
                .or_else(|| spring.map(bounce)),
            _ => None,
        }
    }

    /// The CSS timing function of a segment lasting `seconds`.
    fn css(&self, seconds: f64, notes: &mut Notes) -> String {
        match self {
            Easing::Linear => "linear".into(),
            Easing::Hold => "steps(1, jump-end)".into(),
            Easing::Cubic(kind, b) => match *kind {
                "IN_CUBIC" | "EASE_IN" => "ease-in".into(),
                "OUT_CUBIC" => "ease-out".into(),
                "INOUT_CUBIC" => "ease-in-out".into(),
                _ => cubic_css(b),
            },
            Easing::Spring(kind, spring, _) => {
                let spring = match (spring, spring_preset(kind)) {
                    (Some(s), _) => {
                        notes.springs_sampled += 1;
                        *s
                    }
                    (None, Some((_, b))) => {
                        notes.springs_sampled += 1;
                        spring_of_bounce(b)
                    }
                    (None, None) => {
                        notes.springs_assumed += 1;
                        DEFAULT_SPRING
                    }
                };
                spring_css(spring, seconds)
            }
        }
    }

    /// The motion.dev `ease` of a segment in a keyframe list. A spring there keeps its stored
    /// curve, as Figma's own snippets do; a two-keyframe spring track uses `type: "spring"`.
    fn motion(&self, notes: &mut Notes) -> String {
        match self {
            Easing::Linear => "\"linear\"".into(),
            Easing::Hold => "() => 0".into(),
            Easing::Cubic(kind, b) => match named_cubic(kind) {
                Some((_, name, _)) => format!("\"{name}\""),
                None => bezier_js(b),
            },
            Easing::Spring(_, _, b) => {
                notes.springs_motion += 1;
                bezier_js(&b.unwrap_or([0.34, 1.56, 0.64, 1.0]))
            }
        }
    }
}

fn cubic_css(b: &[f64; 4]) -> String {
    format!(
        "cubic-bezier({}, {}, {}, {})",
        n(b[0]),
        n(b[1]),
        n(b[2]),
        n(b[3])
    )
}

fn bezier_js(b: &[f64; 4]) -> String {
    format!("[{}, {}, {}, {}]", n(b[0]), n(b[1]), n(b[2]), n(b[3]))
}

/// A damped spring from 0 to 1 sampled over `seconds`, as a CSS `linear()` function.
fn spring_css(s: Spring, seconds: f64) -> String {
    let mut out = String::from("linear(");
    for i in 0..=SPRING_SAMPLES {
        let t = seconds * i as f64 / SPRING_SAMPLES as f64;
        let v = if i == SPRING_SAMPLES {
            1.0
        } else {
            spring_at(s, t)
        };
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&n(v));
    }
    out.push(')');
    out
}

/// Position at `t` seconds of a spring released at 0 towards 1 with no initial velocity.
fn spring_at(s: Spring, t: f64) -> f64 {
    let mass = s.mass.max(1e-6);
    let w0 = (s.stiffness.max(0.0) / mass).sqrt();
    let zeta = s.damping / (2.0 * (s.stiffness.max(1e-9) * mass).sqrt());
    if w0 == 0.0 {
        return 0.0;
    }
    if zeta < 1.0 {
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * w0 * t).exp() * ((wd * t).cos() + zeta * w0 / wd * (wd * t).sin())
    } else if zeta == 1.0 {
        1.0 - (-w0 * t).exp() * (1.0 + w0 * t)
    } else {
        let r = w0 * (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-zeta * w0 + r, -zeta * w0 - r);
        1.0 - (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r2 - r1)
    }
}

/// CSS property and value of `field`, or `None` when CSS has no counterpart.
fn css_value(field: &str, value: &Value, rotation: f64) -> Option<(&'static str, String)> {
    let num = |v: &Value| v.as_f64();
    let xy = |v: &Value| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
    Some(match field {
        "OPACITY" => ("opacity", n(num(value)? / 100.0)),
        "WIDTH" => ("width", format!("{}px", n(num(value)?))),
        "HEIGHT" => ("height", format!("{}px", n(num(value)?))),
        "MOTION_ROTATION" => (
            "rotate",
            format!("{}deg", n(rotation + num(value)?.to_degrees())),
        ),
        "MOTION_TRANSLATION_XY" => {
            let (x, y) = xy(value)?;
            ("translate", format!("{}px {}px", n(x), n(y)))
        }
        "MOTION_SCALE_XY" => {
            let (x, y) = xy(value)?;
            ("scale", format!("{} {}", n(x), n(y)))
        }
        _ => return None,
    })
}

/// The motion.dev values of `field` (one or two names).
fn motion_values(field: &str, value: &Value, rotation: f64) -> Option<Vec<(&'static str, String)>> {
    let num = |v: &Value| v.as_f64();
    let xy = |v: &Value| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
    Some(match field {
        "OPACITY" => vec![("opacity", n(num(value)? / 100.0))],
        "WIDTH" => vec![("width", n(num(value)?))],
        "HEIGHT" => vec![("height", n(num(value)?))],
        "MOTION_ROTATION" => vec![("rotate", n(rotation + num(value)?.to_degrees()))],
        // Figma stores shear in radians; motion.dev's skewX is in degrees.
        "MOTION_SHEAR" => vec![("skewX", n(num(value)?.to_degrees()))],
        "MOTION_TRANSLATION_X" => vec![("x", n(num(value)?))],
        "MOTION_TRANSLATION_Y" => vec![("y", n(num(value)?))],
        "MOTION_SCALE_X" => vec![("scaleX", n(num(value)?))],
        "MOTION_SCALE_Y" => vec![("scaleY", n(num(value)?))],
        "MOTION_TRANSLATION_XY" => {
            let (x, y) = xy(value)?;
            vec![("x", n(x)), ("y", n(y))]
        }
        "MOTION_SCALE_XY" => {
            let (x, y) = xy(value)?;
            vec![("scaleX", n(x)), ("scaleY", n(y))]
        }
        _ => return None,
    })
}

/// Keyframes over the whole timeline: a hold from 0 to the first keyframe, the keyframes, a
/// hold to the end. Each is (time µs, value, easing of the segment ending there).
fn padded(b: &Binding, duration: f64) -> Vec<(f64, Value, Easing)> {
    let mut out = Vec::new();
    let first = &b.keyframes[0];
    if first.time_us > 0.0 {
        out.push((0.0, first.value.clone(), Easing::Linear));
    }
    for (i, k) in b.keyframes.iter().enumerate() {
        // The first keyframe's easing shapes nothing: before it the value holds.
        let easing = if i == 0 {
            Easing::Linear
        } else {
            k.easing.clone()
        };
        out.push((k.time_us, k.value.clone(), easing));
    }
    let last = b.keyframes.last().map_or(0.0, |k| k.time_us);
    if last < duration {
        out.push((
            duration,
            b.keyframes.last().unwrap().value.clone(),
            Easing::Linear,
        ));
    }
    out
}

/// The JSON entry of one animated node: bindings and code snippets.
fn node_entry(a: &Animated, notes: &mut Notes) -> Value {
    let class = format!(
        "n{}",
        a.id.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
    );
    let duration = a.duration_us.max(1.0);
    let bindings: Vec<Value> = a
        .bindings
        .iter()
        .map(|b| {
            if b.curved {
                notes.curved_paths += 1;
            }
            json!({
                "field": b.field,
                "operation": b.operation,
                "preset": b.preset,
                "delayMs": ms(b.keyframes[0].time_us),
                "keyframes": b.keyframes.iter().map(|k| json!({
                    "timeMs": ms(k.time_us),
                    "value": tidy(k.value.clone()),
                    "easing": k.easing.json(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();

    let (css, motion) = snippets(a, &class, duration, notes);
    let mut entry = json!({
        "nodeId": a.id,
        "name": a.name,
        "nodeType": a.kind,
        "timelineRootId": a.root,
        "durationMs": ms(a.duration_us),
        "loopMode": a.loop_mode,
        "keyframeBindings": bindings,
        "codeSnippets": {"css": css, "motion": motion},
    });
    if let Some(origin) = &a.origin {
        entry["transformOrigin"] = json!(origin);
    }
    if let Some(text) = &a.text {
        entry["textAnimation"] = text.clone();
    }
    let summary: Vec<String> = a
        .bindings
        .iter()
        .filter(|b| {
            !is_trim(&b.field) && motion_values(&b.field, &b.keyframes[0].value, 0.0).is_none()
        })
        .map(|b| {
            format!(
                "{} from {} to {} between {} and {} ms",
                b.field,
                tidy(b.keyframes[0].value.clone()),
                tidy(b.keyframes.last().unwrap().value.clone()),
                ms(b.keyframes[0].time_us),
                ms(b.keyframes.last().unwrap().time_us)
            )
        })
        .collect();
    if !summary.is_empty() {
        entry["motionSummary"] = json!(summary.join("; "));
    }
    entry
}

/// Snippet keyframes: time (µs), value as written in code, easing of the segment ending there.
type Frames = Vec<(f64, String, Easing)>;

/// One motion.dev value: its name, keyframes, and for a two-keyframe spring track the spring
/// (bounce, start µs, end µs) that replaces its keyframe timing.
struct MotionValue {
    name: &'static str,
    frames: Frames,
    spring: Option<(f64, f64, f64)>,
}

/// CSS `@keyframes` (one animation per property) and a motion.dev element in the shape of
/// Figma's snippets (`initial`, `animate`, `transition`).
fn snippets(a: &Animated, class: &str, duration: f64, notes: &mut Notes) -> (String, String) {
    (
        css_snippet(a, class, duration, notes),
        motion_snippet(a, duration, notes),
    )
}

fn css_snippet(a: &Animated, class: &str, duration: f64, notes: &mut Notes) -> String {
    let iteration = match a.loop_mode {
        "loop" => " infinite",
        "boomerang" => " infinite alternate",
        _ => " both",
    };
    // CSS: x and y tracks become one translate/scale property.
    let mut css_tracks: Vec<(&'static str, Frames)> = Vec::new();
    let mut axes: HashMap<&'static str, [Option<&Binding>; 2]> = HashMap::new();
    for b in &a.bindings {
        let axis = match b.field.as_str() {
            "MOTION_TRANSLATION_X" => Some(("translate", 0)),
            "MOTION_TRANSLATION_Y" => Some(("translate", 1)),
            "MOTION_SCALE_X" => Some(("scale", 0)),
            "MOTION_SCALE_Y" => Some(("scale", 1)),
            _ => None,
        };
        if let Some((property, i)) = axis {
            axes.entry(property).or_default()[i] = Some(b);
            continue;
        }
        let frames: Option<Vec<_>> = padded(b, duration)
            .into_iter()
            .map(|(t, v, e)| css_value(&b.field, &v, a.base_rotation).map(|(_, s)| (t, s, e)))
            .collect();
        match (
            frames,
            css_value(&b.field, &b.keyframes[0].value, a.base_rotation),
        ) {
            (Some(frames), Some((property, _))) => css_tracks.push((property, frames)),
            _ => {
                let list = if motion_values(&b.field, &b.keyframes[0].value, 0.0).is_some()
                    || is_trim(&b.field)
                {
                    &mut notes.css_missing
                } else {
                    &mut notes.unconverted
                };
                if !list.contains(&b.field) {
                    list.push(b.field.clone());
                }
            }
        }
    }
    let mut properties: Vec<_> = axes.into_iter().collect();
    properties.sort_by_key(|(p, _)| *p);
    for (property, [x, y]) in properties {
        let identity = if property == "scale" { 1.0 } else { 0.0 };
        let unit = if property == "translate" { "px" } else { "" };
        let frames = merge_axes(x, y, duration, (identity, identity), notes);
        let frames = frames
            .into_iter()
            .map(|(t, vx, vy, e)| (t, format!("{}{unit} {}{unit}", n(vx), n(vy)), e))
            .collect();
        css_tracks.push((property, frames));
    }

    let mut css = format!("/* {} \"{}\" */\n", a.id, a.name.replace("*/", "* /"));
    let mut animations = Vec::new();
    for (property, frames) in &css_tracks {
        let name = format!("{class}-{property}");
        let _ = writeln!(css, "@keyframes {name} {{");
        for (i, (t, value, _)) in frames.iter().enumerate() {
            let next = frames.get(i + 1);
            let timing = next.map(|(t2, _, e)| e.css((t2 - t) / 1e6, notes));
            let _ = write!(
                css,
                "  {}% {{ {property}: {value};",
                n(t / duration * 100.0)
            );
            if let Some(timing) = timing.filter(|f| f != "linear") {
                let _ = write!(css, " animation-timing-function: {timing};");
            }
            css.push_str(" }\n");
        }
        css.push_str("}\n");
        animations.push(format!("{name} {}ms{iteration}", n(ms(duration))));
    }
    if !animations.is_empty() {
        let _ = writeln!(css, ".{class} {{");
        if let Some(origin) = &a.origin {
            let _ = writeln!(css, "  transform-origin: {origin};");
        }
        let _ = writeln!(css, "  animation: {};\n}}", animations.join(",\n    "));
    }
    css
}

fn motion_snippet(a: &Animated, duration: f64, notes: &mut Notes) -> String {
    let mut values: Vec<MotionValue> = Vec::new();
    let (mut start, mut end) = (None, None);
    for b in &a.bindings {
        match b.field.as_str() {
            "PATH_TRIM_START" => {
                start = Some(b);
                continue;
            }
            "PATH_TRIM_END" => {
                end = Some(b);
                continue;
            }
            _ => {}
        }
        let Some(first) = motion_values(&b.field, &b.keyframes[0].value, a.base_rotation) else {
            continue;
        };
        // A two-keyframe spring track is a real spring (Figma's snippets never bake it into
        // a curve); longer tracks keep the stored curve per segment.
        let spring = (b.keyframes.len() == 2)
            .then(|| b.keyframes[1].easing.bounce())
            .flatten()
            .map(|bounce| (bounce, b.keyframes[0].time_us, b.keyframes[1].time_us));
        let frames: Vec<(f64, Value, Easing)> = if spring.is_some() {
            b.keyframes
                .iter()
                .map(|k| (k.time_us, k.value.clone(), k.easing.clone()))
                .collect()
        } else {
            padded(b, duration)
        };
        for (slot, (name, _)) in first.iter().enumerate() {
            let track: Option<Frames> = frames
                .iter()
                .map(|(t, v, e)| {
                    motion_values(&b.field, v, a.base_rotation)
                        .map(|vals| (*t, vals[slot].1.clone(), e.clone()))
                })
                .collect();
            if let Some(frames) = track {
                values.push(MotionValue {
                    name,
                    frames,
                    spring,
                });
            }
        }
    }
    // Path trim: a stroke dash over `pathLength={1}`, as Figma's path-draw snippets.
    let trim = start.is_some() || end.is_some();
    if trim {
        let merged = merge_axes(start, end, duration, a.trim, notes);
        values.push(MotionValue {
            name: "strokeDasharray",
            frames: merged
                .iter()
                .map(|(t, s, e, ease)| (*t, format!("'{} 1'", n((e - s).max(0.0))), ease.clone()))
                .collect(),
            spring: None,
        });
        if start.is_some() || a.trim.0 != 0.0 {
            values.push(MotionValue {
                name: "strokeDashoffset",
                frames: merged
                    .iter()
                    .map(|(t, s, _, ease)| (*t, n(-s), ease.clone()))
                    .collect(),
                spring: None,
            });
        }
    }

    let tag = if trim {
        "motion.path"
    } else if a.vector {
        "motion.svg"
    } else {
        "motion.div"
    };
    let repeat = match a.loop_mode {
        "loop" => ", repeat: Infinity",
        "boomerang" => ", repeat: Infinity, repeatType: \"reverse\"",
        _ => "",
    };
    let seconds = |us: f64| n(us / 1e6);
    let keyframed = |v: &MotionValue, notes: &mut Notes| -> String {
        let times: Vec<_> = v.frames.iter().map(|(t, _, _)| n(t / duration)).collect();
        let eases: Vec<_> = v
            .frames
            .iter()
            .skip(1)
            .map(|(_, _, e)| e.motion(notes))
            .collect();
        format!(
            "duration: {}, times: [{}], ease: [{}]{repeat}",
            seconds(duration),
            times.join(", "),
            eases.join(", ")
        )
    };
    let mut out = format!("<{tag}\n");
    if trim {
        out.push_str("  pathLength={1}\n");
    }
    let list =
        |f: &dyn Fn(&MotionValue) -> String| values.iter().map(f).collect::<Vec<_>>().join(", ");
    let _ = writeln!(
        out,
        "  initial={{{{ {} }}}}",
        list(&|v| format!("{}: {}", v.name, v.frames[0].1))
    );
    let _ = writeln!(
        out,
        "  animate={{{{ {} }}}}",
        list(&|v| format!(
            "{}: [{}]",
            v.name,
            v.frames
                .iter()
                .map(|(_, s, _)| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    );
    if let Some(origin) = &a.origin {
        let _ = writeln!(out, "  style={{{{ transformOrigin: \"{origin}\" }}}}");
    }
    // One shared transition when every value has the same keyframe timing, else one per value.
    let timings: Vec<String> = values
        .iter()
        .map(|v| match v.spring {
            Some((bounce, t0, t1)) => {
                let mut s = format!(
                    "type: \"spring\", bounce: {}, duration: {}, delay: {}{repeat}",
                    n(bounce),
                    seconds(t1 - t0),
                    seconds(t0)
                );
                if !repeat.is_empty() && duration > t1 {
                    let _ = write!(s, ", repeatDelay: {}", seconds(duration - t1 + t0));
                }
                s
            }
            None => keyframed(v, notes),
        })
        .collect();
    if timings.windows(2).all(|w| w[0] == w[1]) && !timings.is_empty() {
        let _ = writeln!(out, "  transition={{{{ {} }}}}", timings[0]);
    } else {
        out.push_str("  transition={{\n");
        for (v, t) in values.iter().zip(&timings) {
            let _ = writeln!(out, "    {}: {{ {t} }},", v.name);
        }
        out.push_str("  }}\n");
    }
    out.push_str("/>\n");
    out
}

/// One CSS track from an x and a y binding: the union of their keyframe times, each axis
/// interpolated linearly where it has no keyframe. `identity` stands in for a missing axis.
fn merge_axes(
    x: Option<&Binding>,
    y: Option<&Binding>,
    duration: f64,
    identity: (f64, f64),
    notes: &mut Notes,
) -> Vec<(f64, f64, f64, Easing)> {
    let frames = |b: Option<&Binding>, id: f64| -> Vec<(f64, f64, Easing)> {
        b.map(|b| {
            padded(b, duration)
                .into_iter()
                .map(|(t, v, e)| (t, v.as_f64().unwrap_or(id), e))
                .collect()
        })
        .unwrap_or_default()
    };
    let (fx, fy) = (frames(x, identity.0), frames(y, identity.1));
    let times = |f: &[(f64, f64, Easing)]| f.iter().map(|k| k.0).collect::<Vec<_>>();
    if !fx.is_empty() && !fy.is_empty() && times(&fx) != times(&fy) {
        notes.merged_axes += 1;
    }
    let mut all: Vec<f64> = fx.iter().chain(&fy).map(|k| k.0).collect();
    all.sort_by(f64::total_cmp);
    all.dedup();
    let at = |f: &[(f64, f64, Easing)], t: f64, identity: f64| -> f64 {
        if f.is_empty() {
            return identity;
        }
        match f.iter().position(|k| k.0 >= t) {
            Some(0) => f[0].1,
            Some(i) if f[i].0 == t => f[i].1,
            Some(i) => {
                let (a, b) = (&f[i - 1], &f[i]);
                a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0)
            }
            None => f[f.len() - 1].1,
        }
    };
    let ease = |f: &[(f64, f64, Easing)], t: f64| f.iter().find(|k| k.0 == t).map(|k| k.2.clone());
    all.into_iter()
        .map(|t| {
            let e = ease(&fx, t)
                .or_else(|| ease(&fy, t))
                .unwrap_or(Easing::Linear);
            (t, at(&fx, t, identity.0), at(&fy, t, identity.1), e)
        })
        .collect()
}

/// Timeline definitions by ID, with the node that holds each.
struct Timelines {
    by_id: HashMap<Guid, (Guid, f64, &'static str)>,
}

impl Timelines {
    fn new(doc: &Document) -> Self {
        let mut by_id = HashMap::new();
        for node in &doc.nodes {
            for entry in node
                .data
                .path(&["timelineDefinitions", "entries"])
                .map_or(&[][..], Val::items)
            {
                if let Some(id) = entry.get("id").and_then(Val::as_guid) {
                    let data = entry.get("data");
                    let duration = data
                        .and_then(|d| d.get("durationUs"))
                        .and_then(number)
                        .filter(|d| *d > 0.0)
                        .or_else(|| node.data.get("timelineDuration").and_then(number))
                        .unwrap_or(0.0);
                    by_id.insert(id, (node.id, duration, loop_mode(data)));
                }
            }
        }
        Timelines { by_id }
    }

    /// Root node, duration (µs) and loop mode of the timeline `node` plays in.
    fn of(&self, doc: &Document, node: Guid, named: Option<Guid>) -> (String, f64, &'static str) {
        if let Some((root, duration, mode)) = named.and_then(|id| self.by_id.get(&id)) {
            return (root.to_string(), *duration, mode);
        }
        let mut current = Some(node);
        let mut top = node;
        let mut steps = 0;
        while let Some(id) = current {
            steps += 1;
            let Some(n) = doc.node(id) else { break };
            if n.kind == "CANVAS" || n.kind == "DOCUMENT" || steps > 10_000 {
                break;
            }
            let entries = n
                .data
                .path(&["timelineDefinitions", "entries"])
                .map_or(&[][..], Val::items);
            let default = entries
                .iter()
                .find(|e| e.path(&["data", "defaultTimeline"]).and_then(Val::as_bool) == Some(true))
                .or_else(|| entries.first());
            if let Some((root, duration, mode)) = default
                .and_then(|e| e.get("id"))
                .and_then(Val::as_guid)
                .and_then(|id| self.by_id.get(&id))
            {
                return (root.to_string(), *duration, mode);
            }
            if let Some(duration) = n.data.get("timelineDuration").and_then(number) {
                return (id.to_string(), duration, "once");
            }
            top = id;
            current = n.parent;
        }
        // No timeline stored: the top-level frame, as long as its last keyframe.
        (top.to_string(), 0.0, "once")
    }
}

fn loop_mode(data: Option<&Val>) -> &'static str {
    match data.and_then(|d| d.text("playbackStyle")) {
        Some("LOOP") => "loop",
        Some("BOOMERANG") => "boomerang",
        _ => "once",
    }
}

/// A stored integer or float as f64.
fn number(v: &Val) -> Option<f64> {
    match v {
        Val::Int(i) => Some(*i as f64),
        Val::Float(f) if f.is_finite() => Some(*f),
        _ => None,
    }
}

/// Microseconds as milliseconds, three decimals.
fn ms(us: f64) -> f64 {
    (us / 1e3 * 1e3).round() / 1e3
}

/// `v` with at most four decimals and no trailing zeros.
fn n(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let mut s = format!("{v:.4}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" { "0".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_underdamped_spring_overshoots_and_settles() {
        let s = Spring {
            stiffness: 100.0,
            damping: 6.5,
            mass: 1.0,
        };
        assert_eq!(spring_at(s, 0.0), 0.0);
        let peak = (1..200)
            .map(|i| spring_at(s, f64::from(i) / 100.0))
            .fold(0.0, f64::max);
        assert!(peak > 1.2, "{peak}");
        assert!((spring_at(s, 5.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn operations_combine_with_the_base_value() {
        assert_eq!(
            combine("SCALE", Some(json!(100.0)), json!(0.5)),
            json!(50.0)
        );
        assert_eq!(
            combine("OFFSET", Some(json!(10.0)), json!(5.0)),
            json!(15.0)
        );
        assert_eq!(combine("SET", Some(json!(10.0)), json!(5.0)), json!(5.0));
        assert_eq!(
            combine(
                "SCALE",
                Some(json!({"x": 2.0, "y": 3.0})),
                json!({"x": 0.5, "y": 1.0})
            ),
            json!({"x": 1.0, "y": 3.0})
        );
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(n(0.30000001192092896), "0.3");
        assert_eq!(n(-0.0), "0");
        assert_eq!(n(1500.0), "1500");
    }
}
