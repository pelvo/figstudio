//! `list_file_shaders` and `get_shader`: the shader fills and effects a file uses.
//!
//! Figma stores each shader a file uses as a `CODE_COMPONENT` node (`codeObjectType`
//! `CUSTOM_FILL` or `CUSTOM_EFFECT`) holding its parameter definitions and a reference to the
//! published tool. Layers use one through a `CUSTOM` paint or effect whose `customEffectId`
//! points at that node's key, with the parameter values set on the layer. The shader's name,
//! description and source files stay in Figma's cloud: they are not in the `.fig`.
use crate::output::Output;
use figstudio_fig::{Document, Guid, Node, Val};
use serde_json::{Map, Value, json};

/// Most shaders `list_file_shaders` returns (Figma's own limit).
pub const MAX_SHADERS: usize = 100;
/// Most layers `get_shader` lists as using one shader.
pub const MAX_USAGES: usize = 200;

/// The note every reply carries, so a client does not look for what the file does not hold.
pub const OFFLINE_NOTE: &str = "Offline: a .fig stores each shader's parameters and the \
layers that use it, not its name, description or source files, which stay in Figma's cloud.";

/// The note when a shader's source was found in the file's shader folder.
const SOURCE_NOTE: &str = "Name and source files come from the file's shader folder \
(FILE.fig.shaders/: the zip of Figma's \"View code\" → \"Download files\", or its folder).";

/// How to add the source of a shader the folder does not hold.
const MISSING_NOTE: &str = "To add a shader's name and source: in Figma, open the shader's \
\"…\" menu → View code → Download files, and save the zip in FILE.fig.shaders/ next to the \
.fig.";

/// A shader's source as Figma's "View code" → "Download files" gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShaderSource {
    /// Where it was read from (the zip's or folder's name, without extension), used to match
    /// it by shader ID or key.
    pub label: String,
    /// `features.json` `name`.
    pub name: Option<String>,
    /// `features.json` `version`.
    pub version: Option<u64>,
    /// File name and text, `features.json` and `main.ts` among them.
    pub files: Vec<(String, String)>,
}

impl ShaderSource {
    /// Reads name and version from `features.json`.
    pub fn new(label: String, files: Vec<(String, String)>) -> Self {
        let features = files
            .iter()
            .find(|(f, _)| f.rsplit('/').next() == Some("features.json"))
            .and_then(|(_, t)| serde_json::from_str::<Value>(t).ok());
        ShaderSource {
            label,
            name: features
                .as_ref()
                .and_then(|f| f["name"].as_str().map(str::to_owned)),
            version: features.as_ref().and_then(|f| f["version"].as_u64()),
            files,
        }
    }

    /// The property names `main.ts` declares with `defineProperties`.
    pub fn parameters(&self) -> Vec<String> {
        self.files
            .iter()
            .find(|(f, _)| f.rsplit('/').next() == Some("main.ts"))
            .map_or_else(Vec::new, |(_, text)| define_properties(text))
    }
}

/// The top-level keys of the object passed to `defineProperties(…, { … })`.
fn define_properties(text: &str) -> Vec<String> {
    let Some(at) = text.find("defineProperties(") else {
        return Vec::new();
    };
    // Skip the import line: the call is the one followed by a comma and an object.
    let start = text[at..]
        .match_indices("defineProperties(")
        .map(|(i, _)| at + i)
        .find_map(|i| {
            let rest = &text[i..];
            let comma = rest.find(',')?;
            let brace = rest.find('{')?;
            (comma < brace).then_some(i + brace)
        });
    let Some(start) = start else {
        return Vec::new();
    };
    let bytes = text.as_bytes();
    let (mut depth, mut i, mut out) = (0usize, start, Vec::new());
    let mut token = String::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '"' | '\'' | '`' => {
                // Skip a string (a quoted key at depth 1 is still a key).
                let quote = c;
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] as char != quote {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                if depth == 1 {
                    token = text.get(i + 1..j.min(text.len())).unwrap_or("").to_owned();
                }
                i = j + 1;
                continue;
            }
            '{' | '[' | '(' => depth += 1,
            '}' | ']' | ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
            ':' if depth == 1 && !token.is_empty() => {
                out.push(std::mem::take(&mut token));
            }
            c if depth == 1 && (c.is_alphanumeric() || c == '_' || c == '$') => token.push(c),
            ',' | '\n' | ' ' if depth == 1 => token.clear(),
            _ => {}
        }
        i += 1;
    }
    out
}

/// A shader fill or effect stored in the document.
struct Shader<'a> {
    node: &'a Node,
    key: Option<&'a str>,
    kind: &'static str,
}

impl<'a> Shader<'a> {
    fn new(node: &'a Node) -> Option<Self> {
        if node.kind != "CODE_COMPONENT" {
            return None;
        }
        let kind = match node.data.text("codeObjectType")? {
            "CUSTOM_FILL" => "fill",
            "CUSTOM_EFFECT" => "effect",
            _ => return None,
        };
        Some(Shader {
            node,
            key: node.data.text("key"),
            kind,
        })
    }

    /// The published tool's ID, or the stored key for a shader that was never published.
    fn id(&self) -> String {
        self.published_id()
            .map(str::to_owned)
            .or_else(|| self.key.map(str::to_owned))
            .unwrap_or_else(|| self.node.id.to_string())
    }

    fn published_id(&self) -> Option<&'a str> {
        self.node
            .data
            .path(&["customToolArtifactRef", "publishedCustomToolId"])
            .and_then(Val::as_str)
    }

    fn version(&self) -> Option<&'a str> {
        self.node
            .data
            .path(&["customToolArtifactRef", "publishedCustomToolVersionId"])
            .and_then(Val::as_str)
            .or_else(|| self.node.data.text("version"))
    }

    /// Whether the file holds a library's published version (it came from a library).
    fn published(&self) -> bool {
        self.node.data.get("sourceLibraryKey").is_some()
    }

    /// Parameter definitions in the order Figma shows them (`sortPosition`, a fractional
    /// index compared as text).
    fn definitions(&self) -> Vec<&'a Val> {
        let mut defs: Vec<_> = self
            .node
            .data
            .get("componentPropDefs")
            .map_or(&[][..], Val::items)
            .iter()
            .collect();
        defs.sort_by_key(|d| d.text("sortPosition").unwrap_or(""));
        defs
    }

    /// Whether `reference` (a `customEffectId`) points at this shader.
    fn is(&self, reference: &Val) -> bool {
        let by_key = reference
            .path(&["assetRef", "key"])
            .and_then(Val::as_str)
            .is_some_and(|key| Some(key) == self.key);
        let by_guid = reference
            .get("guid")
            .and_then(Val::as_guid)
            .is_some_and(|guid| guid == self.node.id);
        by_key || by_guid
    }

    fn matches(&self, query: &str) -> bool {
        query == self.node.id.to_string()
            || Some(query) == self.key
            || Some(query) == self.published_id()
    }

    /// Its source: one named after its ID, key or node ID, else the only one whose
    /// `defineProperties` names are the shader's parameter names.
    fn source<'s>(&self, sources: &'s [ShaderSource]) -> Option<&'s ShaderSource> {
        let by_label = sources.iter().find(|s| {
            let label = s.label.trim_end_matches("-source");
            self.matches(label) || label == self.node.id.to_string().replace(':', "-")
        });
        if by_label.is_some() {
            return by_label;
        }
        let mut mine: Vec<String> = self
            .definitions()
            .iter()
            .filter_map(|d| d.text("name").map(str::to_owned))
            .collect();
        mine.sort();
        if mine.is_empty() {
            return None;
        }
        let mut found = sources.iter().filter(|s| {
            let mut theirs = s.parameters();
            theirs.sort();
            theirs == mine
        });
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    fn summary(&self, usages: usize) -> Value {
        let parameters: Vec<_> = self
            .definitions()
            .iter()
            .filter_map(|d| d.text("name"))
            .collect();
        json!({
            "id": self.id(),
            "nodeId": self.node.id.to_string(),
            "key": self.key,
            "type": self.kind,
            "version": self.version(),
            "published": self.published(),
            "name": null,
            "description": null,
            "parameters": parameters,
            "usedBy": usages,
            "files": [],
        })
    }
}

fn shaders(doc: &Document) -> Vec<Shader<'_>> {
    doc.nodes.iter().filter_map(Shader::new).collect()
}

/// One layer's use of a shader: where the paint or effect sits and its parameter values.
struct Usage<'a> {
    node: &'a Node,
    /// Field path inside the node, for example `fillPaints[0]` or
    /// `symbolData.symbolOverrides[2].effects[0]`.
    field: String,
    paint: &'a Val,
}

/// Every `CUSTOM` paint or effect in the document whose reference `points` accepts.
fn usages<'a>(doc: &'a Document, points: impl Fn(&Val) -> bool) -> Vec<Usage<'a>> {
    let mut out = Vec::new();
    for node in &doc.nodes {
        if node.kind == "CODE_COMPONENT" {
            continue;
        }
        let mut path = String::new();
        walk(&node.data, &mut path, &mut |field, paint| {
            if paint.get("customEffectId").is_some_and(&points) {
                out.push(Usage {
                    node,
                    field: field.to_owned(),
                    paint,
                });
            }
        });
    }
    out
}

/// Calls `found` with the path of every object holding a `customEffectId`.
fn walk<'v>(value: &'v Val, path: &mut String, found: &mut impl FnMut(&str, &'v Val)) {
    match value {
        Val::Obj(fields) => {
            if fields.contains_key("customEffectId") {
                found(path, value);
                return;
            }
            for (key, child) in fields.iter() {
                if matches!(child, Val::Obj(_) | Val::Arr(_)) {
                    let len = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(key);
                    walk(child, path, found);
                    path.truncate(len);
                }
            }
        }
        Val::Arr(items) => {
            for (i, child) in items.iter().enumerate() {
                if matches!(child, Val::Obj(_) | Val::Arr(_)) {
                    let len = path.len();
                    path.push_str(&format!("[{i}]"));
                    walk(child, path, found);
                    path.truncate(len);
                }
            }
        }
        _ => {}
    }
}

/// The shader fills and effects on one layer: field, type, shader ID, version, visibility and
/// the values it sets, labelled as in Figma's panel (in panel order, defaults filled in).
pub fn on_node(doc: &Document, node: &Node) -> Vec<Value> {
    let mut found = Vec::new();
    let mut path = String::new();
    walk(&node.data, &mut path, &mut |field, paint| {
        found.push((field.to_owned(), paint));
    });
    if found.is_empty() {
        return Vec::new();
    }
    let all = shaders(doc);
    found
        .into_iter()
        .map(|(field, paint)| {
            let shader = paint
                .get("customEffectId")
                .and_then(|r| all.iter().find(|s| s.is(r)));
            let Some(shader) = shader else {
                return json!({"field": field, "type": null, "id": null,
                    "visible": paint.flag("visible") != Some(false), "values": []});
            };
            let set = |def: &Val| -> Option<Value> {
                let id = def.get("id").and_then(Val::as_guid)?;
                paint
                    .get("componentPropAssignments")
                    .map_or(&[][..], Val::items)
                    .iter()
                    .find(|a| a.get("defID").and_then(Val::as_guid) == Some(id))
                    .map(|a| {
                        a.get("varValue")
                            .map(variable_value)
                            .or_else(|| a.get("value").map(plain_value))
                            .unwrap_or(Value::Null)
                    })
            };
            let values: Vec<Value> = shader
                .definitions()
                .iter()
                .map(|def| {
                    let p = parameter(def);
                    let value = set(def).unwrap_or_else(|| p["default"].clone());
                    let label = p["label"].as_str().or(p["name"].as_str()).unwrap_or("?");
                    // A select shows its option's label, not the stored number.
                    let shown = p["options"]
                        .as_array()
                        .and_then(|o| o.iter().find(|o| o["value"] == value))
                        .map_or(value.clone(), |o| o["label"].clone());
                    json!({"label": label, "value": shown})
                })
                .collect();
            json!({
                "field": field,
                "type": shader.kind,
                "id": shader.id(),
                "version": shader.version(),
                "visible": paint.flag("visible") != Some(false),
                "values": values,
            })
        })
        .collect()
}

/// `list_file_shaders`: the shader fills and effects the file holds, with how many layers use
/// each.
pub fn list(doc: &Document) -> Output {
    list_with(doc, &[])
}

/// [`list`] with the sources of the file's shader folder: matched shaders get their name and
/// files.
pub fn list_with(doc: &Document, sources: &[ShaderSource]) -> Output {
    let all = shaders(doc);
    let used = usages(doc, |r| all.iter().any(|s| s.is(r)));
    let total = all.len();
    let list: Vec<_> = all
        .iter()
        .take(MAX_SHADERS)
        .map(|s| {
            let count = used
                .iter()
                .filter(|u| u.paint.get("customEffectId").is_some_and(|r| s.is(r)))
                .count();
            with_source(s.summary(count), s.source(sources), false)
        })
        .collect();
    let found = list.iter().filter(|s| !s["name"].is_null()).count();
    Output::structured(json!({
        "shaders": list,
        "total": total,
        "truncated": total > MAX_SHADERS,
        "note": note(found, total.min(MAX_SHADERS)),
    }))
}

/// `get_shader`: one shader's parameters (label, type, control, range or options, default) and
/// the layers that use it with their values. `id` is the shader ID from `list_file_shaders`,
/// its key, or its node ID.
pub fn get(doc: &Document, id: &str) -> Output {
    get_with(doc, id, &[])
}

/// [`get`] with the sources of the file's shader folder: a matched shader gets its name and
/// its files with their text.
pub fn get_with(doc: &Document, id: &str, sources: &[ShaderSource]) -> Output {
    let id = id.trim();
    let all = shaders(doc);
    let Some(shader) = all.iter().find(|s| s.matches(id)) else {
        return Output::fault(format!(
            "no shader {id} in this file; list_file_shaders lists the ones it uses"
        ));
    };
    let definitions = shader.definitions();
    let names: Vec<(Option<Guid>, &str)> = definitions
        .iter()
        .filter_map(|d| Some((d.get("id").and_then(Val::as_guid), d.text("name")?)))
        .collect();
    let used = usages(doc, |r| shader.is(r));
    let listed: Vec<_> = used
        .iter()
        .take(MAX_USAGES)
        .map(|u| usage(u, &names))
        .collect();
    let source = shader.source(sources);
    let mut out = with_source(shader.summary(used.len()), source, true);
    if let Value::Object(fields) = &mut out {
        fields.insert(
            "parameters".into(),
            definitions.iter().map(|d| parameter(d)).collect(),
        );
        fields.insert("usages".into(), Value::Array(listed));
        fields.insert("usagesTruncated".into(), json!(used.len() > MAX_USAGES));
        fields.insert("note".into(), json!(note(usize::from(source.is_some()), 1)));
    }
    Output::structured(out)
}

/// The shader source a `CUSTOM` paint or effect runs and the `frame.params` its code reads:
/// every parameter by name, the layer's value or else the default.
pub fn program<'s>(
    doc: &Document,
    paint: &Val,
    sources: &'s [ShaderSource],
) -> Option<(&'s ShaderSource, Value)> {
    let reference = paint.get("customEffectId")?;
    let all = shaders(doc);
    let shader = all.iter().find(|s| s.is(reference))?;
    let source = shader.source(sources)?;
    let assignments = paint
        .get("componentPropAssignments")
        .map_or(&[][..], Val::items);
    let mut params = Map::new();
    for def in shader.definitions() {
        let Some(name) = def.text("name") else {
            continue;
        };
        let id = def.get("id").and_then(Val::as_guid);
        let assigned = assignments
            .iter()
            .find(|a| id.is_some() && a.get("defID").and_then(Val::as_guid) == id);
        let value = assigned
            .and_then(|a| {
                a.get("varValue")
                    .map(variable_value)
                    .or_else(|| a.get("value").map(plain_value))
            })
            .or_else(|| def.get("varValue").map(variable_value))
            .or_else(|| def.get("initialValue").map(plain_value))
            .unwrap_or(Value::Null);
        params.insert(name.to_owned(), code_value(value));
    }
    Some((source, Value::Object(params)))
}

/// A stored value as shader code reads it: a circle's `center` becomes its `x` and `y`
/// (`{x, y, radius, angle}`, as `pointAngleRadiusParam` reads it), and values nested as
/// variable data (`{dataType, value: {colorValue: …}}`, a gradient stop's colour) become the
/// plain value.
fn code_value(value: Value) -> Value {
    match value {
        Value::Object(mut fields) => {
            if fields.contains_key("dataType")
                && let Some(Value::Object(inner)) = fields.remove("value")
                && inner.len() == 1
                && let Some((_, v)) = inner.into_iter().next()
            {
                return code_value(v);
            }
            if let Some(Value::Object(center)) = fields.remove("center") {
                for (k, v) in center {
                    fields.entry(k).or_insert(v);
                }
            }
            Value::Object(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, code_value(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(code_value).collect()),
        other => other,
    }
}

/// Most bytes of source text `get_shader` returns (Figma's own limit).
const MAX_SOURCE_BYTES: usize = 1_000_000;

/// `summary` with the source's name and files (`content` too when `text`).
fn with_source(mut summary: Value, source: Option<&ShaderSource>, text: bool) -> Value {
    let Some(source) = source else {
        return summary;
    };
    summary["name"] = json!(source.name);
    summary["sourceVersion"] = json!(source.version);
    let mut budget = MAX_SOURCE_BYTES;
    summary["files"] = source
        .files
        .iter()
        .map(|(name, content)| {
            let mut file = json!({"filename": name, "bytes": content.len()});
            if text && content.len() <= budget {
                budget -= content.len();
                file["content"] = json!(content);
            }
            file
        })
        .collect();
    summary
}

/// The reply's note: what offline lacks, and how to add it when sources are missing.
fn note(found: usize, shown: usize) -> String {
    match (found, shown) {
        (_, 0) => OFFLINE_NOTE.to_owned(),
        (f, s) if f == s => SOURCE_NOTE.to_owned(),
        (0, _) => format!("{OFFLINE_NOTE} {MISSING_NOTE}"),
        _ => format!("{SOURCE_NOTE} {MISSING_NOTE}"),
    }
}

/// A parameter definition: name, label, type, control, default and its range or options.
fn parameter(def: &Val) -> Value {
    let config = def.get("parameterConfig");
    let mut out = Map::new();
    out.insert("name".into(), json!(def.text("name")));
    out.insert(
        "label".into(),
        json!(
            config
                .and_then(|c| c.path(&["label", "value", "textValue"]))
                .and_then(Val::as_str)
        ),
    );
    out.insert("type".into(), json!(def.text("type")));
    out.insert(
        "control".into(),
        json!(config.and_then(|c| c.text("control"))),
    );
    let default = def
        .get("varValue")
        .map(variable_value)
        .or_else(|| def.get("initialValue").map(plain_value))
        .unwrap_or(Value::Null);
    out.insert("default".into(), default);
    if let Some(slider) = config.and_then(|c| c.get("sliderConfig")) {
        for bound in ["min", "max", "step"] {
            if let Some(v) = slider.get(bound) {
                out.insert(bound.into(), variable_value(v));
            }
        }
    }
    if let Some(options) = config.and_then(|c| c.path(&["selectConfig", "options"])) {
        let options: Vec<_> = options
            .items()
            .iter()
            .map(|o| {
                json!({
                    "label": o.text("label"),
                    "value": o.get("value").map_or(Value::Null, variable_value),
                })
            })
            .collect();
        out.insert("options".into(), Value::Array(options));
    }
    Value::Object(out)
}

/// One layer's use: node, field, visibility and the values it sets, by parameter name.
fn usage(u: &Usage<'_>, names: &[(Option<Guid>, &str)]) -> Value {
    let mut values = Map::new();
    for assignment in u
        .paint
        .get("componentPropAssignments")
        .map_or(&[][..], Val::items)
    {
        let def = assignment.get("defID").and_then(Val::as_guid);
        let name = names
            .iter()
            .find(|(id, _)| id.is_some() && *id == def)
            .map_or_else(
                || def.map_or_else(|| "?".to_owned(), |g| g.to_string()),
                |(_, n)| (*n).to_owned(),
            );
        let value = assignment
            .get("varValue")
            .map(variable_value)
            .or_else(|| assignment.get("value").map(plain_value))
            .unwrap_or(Value::Null);
        values.insert(name, value);
    }
    json!({
        "nodeId": u.node.id.to_string(),
        "name": u.node.name,
        "nodeType": u.node.kind,
        "field": u.field,
        "visible": u.paint.flag("visible") != Some(false),
        "values": values,
    })
}

/// The value of a `VariableData` (`{dataType, value: {floatValue: …}}`); an alias to a
/// variable becomes `{"alias": key}`.
fn variable_value(data: &Val) -> Value {
    match data.get("value") {
        Some(value) => plain_value(value),
        None => Value::Null,
    }
}

/// The single entry of a `{floatValue: …}`-style value: numbers rounded (stored as f32),
/// text as a string, other values (colours, points, circles) as their fields.
fn plain_value(value: &Val) -> Value {
    let Val::Obj(fields) = value else {
        return tidy(serde_json::to_value(value).unwrap_or(Value::Null));
    };
    let mut entries = fields.iter();
    let (Some((key, inner)), None) = (entries.next(), entries.next()) else {
        return tidy(serde_json::to_value(value).unwrap_or(Value::Null));
    };
    match key {
        "textValue" | "textDataValue" => inner.text("characters").map_or_else(
            || tidy(serde_json::to_value(inner).unwrap_or(Value::Null)),
            |t| json!(t),
        ),
        "alias" => json!({
            "alias": inner
                .path(&["assetRef", "key"])
                .and_then(Val::as_str)
                .map(str::to_owned)
                .or_else(|| inner.get("guid").and_then(Val::as_guid).map(|g| g.to_string())),
        }),
        _ => tidy(serde_json::to_value(inner).unwrap_or(Value::Null)),
    }
}

/// Rounds every number to six decimals, so f32 values read as written (0.1, not
/// 0.10000000149011612).
pub(crate) fn tidy(value: Value) -> Value {
    match value {
        Value::Number(n) if n.is_f64() => n
            .as_f64()
            .map(|f| (f * 1e6).round() / 1e6)
            .and_then(serde_json::Number::from_f64)
            .map_or(Value::Number(n), Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(tidy).collect()),
        Value::Object(fields) => {
            Value::Object(fields.into_iter().map(|(k, v)| (k, tidy(v))).collect())
        }
        other => other,
    }
}
