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
const ROTATION_NOTE: &str = "rotate: Figma stores radians, counter-clockwise positive; CSS and \
motion.dev use degrees clockwise, so the sign is flipped.";
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
}

/// Accumulates what the snippets could not express exactly.
#[derive(Default)]
struct Notes {
    springs_sampled: usize,
    springs_assumed: usize,
    springs_motion: usize,
    curved_paths: usize,
    unconverted: Vec<String>,
    merged_axes: usize,
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
    for a in &found {
        let entry = node_entry(a, &mut notes);
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
    Some(Animated {
        id: id.to_owned(),
        name: data.text("name").unwrap_or(&node.name).to_owned(),
        kind: node.kind.clone(),
        bindings,
        root,
        duration_us: duration,
        loop_mode,
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
    // Named curves (easings.net cubic forms) unless the file stores the handles.
    let cubic =
        |name: &'static str, default: [f64; 4]| Easing::Cubic(name, stored.unwrap_or(default));
    match data.text("easingType").unwrap_or("LINEAR") {
        "LINEAR" => Easing::Linear,
        "HOLD" => Easing::Hold,
        "IN_CUBIC" => cubic("IN_CUBIC", [0.32, 0.0, 0.67, 0.0]),
        "OUT_CUBIC" => cubic("OUT_CUBIC", [0.33, 1.0, 0.68, 1.0]),
        "INOUT_CUBIC" => cubic("INOUT_CUBIC", [0.65, 0.0, 0.35, 1.0]),
        "IN_BACK_CUBIC" => cubic("IN_BACK_CUBIC", [0.36, 0.0, 0.66, -0.56]),
        "OUT_BACK_CUBIC" => cubic("OUT_BACK_CUBIC", [0.34, 1.56, 0.64, 1.0]),
        "INOUT_BACK_CUBIC" => cubic("INOUT_BACK_CUBIC", [0.68, -0.6, 0.32, 1.6]),
        "EASE_IN" => cubic("EASE_IN", [0.42, 0.0, 1.0, 1.0]),
        "CUSTOM_CUBIC" => cubic("CUSTOM_CUBIC", [0.25, 0.1, 0.25, 1.0]),
        "SPRING" => Easing::Spring("SPRING", spring, stored),
        "GENTLE_SPRING" => Easing::Spring("GENTLE_SPRING", spring, stored),
        "CUSTOM_SPRING" => Easing::Spring("CUSTOM_SPRING", spring, stored),
        "SPRING_PRESET_ONE" => Easing::Spring("SPRING_PRESET_ONE", spring, stored),
        "SPRING_PRESET_TWO" => Easing::Spring("SPRING_PRESET_TWO", spring, stored),
        "SPRING_PRESET_THREE" => Easing::Spring("SPRING_PRESET_THREE", spring, stored),
        _ => cubic("CUSTOM_CUBIC", [0.25, 0.1, 0.25, 1.0]),
    }
}

impl Easing {
    fn json(&self) -> Value {
        match self {
            Easing::Linear => json!({"type": "LINEAR"}),
            Easing::Hold => json!({"type": "HOLD"}),
            Easing::Cubic(name, b) => json!({"type": name, "cubicBezier": tidy(json!(b))}),
            Easing::Spring(name, spring, b) => json!({
                "type": name,
                "spring": spring.map(|s| tidy(json!({"stiffness": s.stiffness, "damping": s.damping, "mass": s.mass}))),
                "cubicBezier": b.map(|b| tidy(json!(b))),
            }),
        }
    }

    /// The CSS timing function of a segment lasting `seconds`.
    fn css(&self, seconds: f64, notes: &mut Notes) -> String {
        match self {
            Easing::Linear => "linear".into(),
            Easing::Hold => "steps(1, jump-end)".into(),
            Easing::Cubic(_, b) => cubic_css(b),
            Easing::Spring(_, spring, _) => {
                let spring = match spring {
                    Some(s) => {
                        notes.springs_sampled += 1;
                        *s
                    }
                    None => {
                        notes.springs_assumed += 1;
                        DEFAULT_SPRING
                    }
                };
                spring_css(spring, seconds)
            }
        }
    }

    /// The motion.dev `ease` of a segment.
    fn motion(&self, notes: &mut Notes) -> String {
        match self {
            Easing::Linear => "\"linear\"".into(),
            Easing::Hold => "() => 0".into(),
            Easing::Cubic(_, b) => bezier_js(b),
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
fn css_value(field: &str, value: &Value) -> Option<(&'static str, String)> {
    let num = |v: &Value| v.as_f64();
    let xy = |v: &Value| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
    Some(match field {
        "OPACITY" => ("opacity", n(num(value)? / 100.0)),
        "WIDTH" => ("width", format!("{}px", n(num(value)?))),
        "HEIGHT" => ("height", format!("{}px", n(num(value)?))),
        "MOTION_ROTATION" => ("rotate", format!("{}deg", n(-num(value)?.to_degrees()))),
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
fn motion_values(field: &str, value: &Value) -> Option<Vec<(&'static str, String)>> {
    let num = |v: &Value| v.as_f64();
    let xy = |v: &Value| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
    Some(match field {
        "OPACITY" => vec![("opacity", n(num(value)? / 100.0))],
        "WIDTH" => vec![("width", n(num(value)?))],
        "HEIGHT" => vec![("height", n(num(value)?))],
        "MOTION_ROTATION" => vec![("rotate", n(-num(value)?.to_degrees()))],
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
    let summary: Vec<String> = a
        .bindings
        .iter()
        .filter(|b| motion_values(&b.field, &b.keyframes[0].value).is_none())
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

/// CSS `@keyframes` (one animation per property) and a motion.dev `animate` call.
fn snippets(a: &Animated, class: &str, duration: f64, notes: &mut Notes) -> (String, String) {
    let iteration = match a.loop_mode {
        "loop" => " infinite",
        "boomerang" => " infinite alternate",
        _ => " both",
    };
    // CSS: x and y tracks become one translate/scale/transform-origin property.
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
            .map(|(t, v, e)| css_value(&b.field, &v).map(|(_, s)| (t, s, e)))
            .collect();
        match (frames, css_value(&b.field, &b.keyframes[0].value)) {
            (Some(frames), Some((property, _))) => css_tracks.push((property, frames)),
            _ => {
                if !notes.unconverted.contains(&b.field) {
                    notes.unconverted.push(b.field.clone());
                }
            }
        }
    }
    let mut properties: Vec<_> = axes.into_iter().collect();
    properties.sort_by_key(|(p, _)| *p);
    for (property, [x, y]) in properties {
        let identity = if property == "scale" { 1.0 } else { 0.0 };
        let unit = if property == "translate" { "px" } else { "" };
        let frames = merge_axes(x, y, duration, identity, notes);
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
        let _ = writeln!(
            css,
            ".{class} {{\n  animation: {};\n}}",
            animations.join(",\n    ")
        );
    }

    // motion.dev: every value has its own keyframes, times and eases.
    let mut values: Vec<(&'static str, Frames)> = Vec::new();
    for b in &a.bindings {
        let frames = padded(b, duration);
        let Some(first) = motion_values(&b.field, &frames[0].1) else {
            continue;
        };
        for (slot, (name, _)) in first.iter().enumerate() {
            let track: Option<Vec<_>> = frames
                .iter()
                .map(|(t, v, e)| {
                    motion_values(&b.field, v).map(|vals| (*t, vals[slot].1.clone(), e.clone()))
                })
                .collect();
            if let Some(track) = track {
                values.push((name, track));
            }
        }
    }
    let mut motion = String::from("import { animate } from \"motion\"\n\n");
    let _ = writeln!(
        motion,
        "animate(\n  document.querySelector(\".{class}\"),\n  {{"
    );
    for (name, frames) in &values {
        let list: Vec<_> = frames.iter().map(|(_, v, _)| v.as_str()).collect();
        let _ = writeln!(motion, "    {name}: [{}],", list.join(", "));
    }
    motion.push_str("  },\n  {\n");
    let _ = writeln!(motion, "    duration: {},", n(duration / 1e6));
    match a.loop_mode {
        "loop" => motion.push_str("    repeat: Infinity,\n    repeatType: \"loop\",\n"),
        "boomerang" => motion.push_str("    repeat: Infinity,\n    repeatType: \"reverse\",\n"),
        _ => {}
    }
    for (name, frames) in &values {
        let times: Vec<_> = frames.iter().map(|(t, _, _)| n(t / duration)).collect();
        let eases: Vec<_> = frames
            .iter()
            .skip(1)
            .map(|(_, _, e)| e.motion(notes))
            .collect();
        let _ = writeln!(
            motion,
            "    {name}: {{ times: [{}], ease: [{}] }},",
            times.join(", "),
            eases.join(", ")
        );
    }
    motion.push_str("  }\n)\n");
    (css, motion)
}

/// One CSS track from an x and a y binding: the union of their keyframe times, each axis
/// interpolated linearly where it has no keyframe. `identity` stands in for a missing axis.
fn merge_axes(
    x: Option<&Binding>,
    y: Option<&Binding>,
    duration: f64,
    identity: f64,
    notes: &mut Notes,
) -> Vec<(f64, f64, f64, Easing)> {
    let frames = |b: Option<&Binding>| -> Vec<(f64, f64, Easing)> {
        b.map(|b| {
            padded(b, duration)
                .into_iter()
                .map(|(t, v, e)| (t, v.as_f64().unwrap_or(identity), e))
                .collect()
        })
        .unwrap_or_default()
    };
    let (fx, fy) = (frames(x), frames(y));
    let times = |f: &[(f64, f64, Easing)]| f.iter().map(|k| k.0).collect::<Vec<_>>();
    if !fx.is_empty() && !fy.is_empty() && times(&fx) != times(&fy) {
        notes.merged_axes += 1;
    }
    let mut all: Vec<f64> = fx.iter().chain(&fy).map(|k| k.0).collect();
    all.sort_by(f64::total_cmp);
    all.dedup();
    let at = |f: &[(f64, f64, Easing)], t: f64| -> f64 {
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
            (t, at(&fx, t), at(&fy, t), e)
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
