//! Sparse metadata XML for pages and local node subtrees.
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use figstudio_fig::{Document, Guid, Node, Val};
use quick_xml::{
    Writer,
    events::{BytesStart, BytesText, Event},
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Inputs for sparse metadata retrieval.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataInput {
    /// Node ID or Figma URL; omit for the page list.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Maximum child depth, with the requested root at depth zero.
    depth: Option<u32>,
}

#[tool_router(router = metadata_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get sparse XML metadata: node IDs, names, types, parent-relative positions and sizes. Rotation is degrees, counter-clockwise positive (Figma convention). Call without nodeId for visible pages. Canvas/document elements omit geometry; unsized nodes omit dimensions. Instances remain leaves; depth limits children. Output caps at 5000 nodes: truncated=\"true\" on <metadata> or <pages> marks omitted nodes. Text caps at 200 characters plus an ellipsis when cut. Use get_design_context for implementation details when available."
    )]
    async fn get_metadata(&self, Parameters(input): Parameters<MetadataInput>) -> CallToolResult {
        let document = match self.files().open(input.file_key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            metadata(&document, input.node_id.as_deref(), input.depth)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// Return a page list or bounded subtree as XML; invalid IDs include the page list.
pub fn metadata(doc: &Document, node_id: Option<&str>, depth: Option<u32>) -> CallToolResult {
    let root = match node_id {
        None => None,
        Some(input) => match parse_node_id(input) {
            Ok(NodeId::Node(id)) if doc.node(id).is_some() => Some(id),
            _ => {
                let pages = xml(doc, None, None).unwrap_or_else(|error| error.to_string());
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "Unknown, invalid or unresolved node ID. Available pages:\n{pages}"
                ))]);
            }
        },
    };
    match xml(doc, root, depth) {
        Ok(xml) => CallToolResult::success(vec![ContentBlock::text(xml)]),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
            "could not write metadata XML: {error}"
        ))]),
    }
}

// Bounds node output and stack memory even for deeply nested or very wide documents.
const MAX_ELEMENTS: usize = 5_000;
const MAX_TEXT_CHARS: usize = 200;

enum Step {
    Visit(Guid, u32),
    Children(Guid, u32, usize),
    End(String),
}

fn xml(doc: &Document, root: Option<Guid>, depth: Option<u32>) -> std::io::Result<String> {
    let mut writer = Writer::new(Vec::new());
    if let Some(root) = root {
        subtree(doc, root, depth, &mut writer)?;
    } else {
        pages(doc, &mut writer)?;
    }
    String::from_utf8(writer.into_inner()).map_err(std::io::Error::other)
}

fn pages(doc: &Document, writer: &mut Writer<Vec<u8>>) -> std::io::Result<()> {
    let visible: Vec<_> = doc
        .pages
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|node| {
            node.data.flag("visible") != Some(false) && node.data.flag("internalOnly") != Some(true)
        })
        .take(MAX_ELEMENTS + 1)
        .collect();
    let mut pages = BytesStart::new("pages");
    if visible.len() > MAX_ELEMENTS {
        pages.push_attribute(("truncated", "true"));
    }
    writer.write_event(Event::Start(pages))?;
    for node in visible.into_iter().take(MAX_ELEMENTS) {
        let mut tag = BytesStart::new("canvas");
        let id = node.id.to_string();
        tag.push_attribute(("id", id.as_str()));
        let name = clean_xml(&node.name);
        tag.push_attribute(("name", name.as_str()));
        writer.write_event(Event::Empty(tag))?;
    }
    writer.write_event(Event::End(BytesStart::new("pages").to_end()))?;
    Ok(())
}

fn subtree(
    doc: &Document,
    root: Guid,
    depth: Option<u32>,
    writer: &mut Writer<Vec<u8>>,
) -> std::io::Result<()> {
    let mut body = Writer::new(Vec::new());
    let mut stack = vec![Step::Visit(root, 0)];
    let mut count = 0;
    let mut truncated = false;
    let mut instances = 0;
    while let Some(step) = stack.pop() {
        match step {
            Step::End(kind) => body.write_event(Event::End(BytesStart::new(kind).to_end()))?,
            Step::Children(id, level, index) => {
                if let Some(child) = doc.children(id).get(index) {
                    if doc.node(*child).is_some_and(is_internal) {
                        stack.push(Step::Children(id, level, index + 1));
                        continue;
                    }
                    if count == MAX_ELEMENTS {
                        truncated = true;
                        continue;
                    }
                    stack.push(Step::Children(id, level, index + 1));
                    stack.push(Step::Visit(*child, level));
                }
            }
            Step::Visit(id, level) => {
                if count == MAX_ELEMENTS {
                    truncated = true;
                    continue;
                }
                let Some(node) = doc.node(id) else {
                    continue;
                };
                if is_internal(node) {
                    continue;
                }
                count += 1;
                let kind = node.kind.to_ascii_lowercase().replace('_', "-");
                let has_children = node.kind != "INSTANCE"
                    && depth.is_none_or(|limit| level < limit)
                    && doc
                        .children(id)
                        .iter()
                        .filter_map(|id| doc.node(*id))
                        .any(|child| !is_internal(child));
                if write_node(&mut body, node, &kind, has_children)? {
                    stack.push(Step::End(kind));
                }
                if has_children {
                    stack.push(Step::Children(id, level + 1, 0));
                }
                if node.kind == "INSTANCE" {
                    instances += 1;
                }
            }
        }
    }
    let mut wrapper = BytesStart::new("metadata");
    if truncated {
        wrapper.push_attribute(("truncated", "true"));
    }
    writer.write_event(Event::Start(wrapper))?;
    writer.get_mut().extend(body.into_inner());
    writer.write_event(Event::End(BytesStart::new("metadata").to_end()))?;
    if instances > 0 {
        let note =
            format!("approximations: {instances} instance leaves (expansion deferred to L5)");
        writer.write_event(Event::Comment(BytesText::new(&note)))?;
    }
    Ok(())
}

fn write_node(
    writer: &mut Writer<Vec<u8>>,
    node: &Node,
    kind: &str,
    children: bool,
) -> std::io::Result<bool> {
    let attrs = attributes(node);
    let mut tag = BytesStart::new(kind);
    for (key, value) in &attrs {
        tag.push_attribute((*key, value.as_str()));
    }
    let text = node_text(node);
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

fn attributes(node: &Node) -> Vec<(&'static str, String)> {
    let num = |field: &str, key: &str, default: f64| {
        node.data
            .get(field)
            .and_then(|value| value.num(key))
            .map_or(default, f64::from)
    };
    let mut attrs = vec![("id", node.id.to_string()), ("name", clean_xml(&node.name))];
    if !matches!(node.kind.as_str(), "CANVAS" | "DOCUMENT") {
        attrs.extend([
            ("x", number(num("transform", "m02", 0.0))),
            ("y", number(num("transform", "m12", 0.0))),
        ]);
        if node.data.get("size").is_some() {
            attrs.extend([
                ("width", number(num("size", "x", 0.0))),
                ("height", number(num("size", "y", 0.0))),
            ]);
        }
    }
    if node.data.flag("visible") == Some(false) {
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
    if node.kind == "INSTANCE"
        && let Some(id) = node
            .data
            .path(&["symbolData", "symbolID"])
            .and_then(Val::as_guid)
    {
        attrs.push(("componentId", id.to_string()));
    }
    attrs
}

fn is_internal(node: &Node) -> bool {
    node.kind == "CANVAS" && node.data.flag("internalOnly") == Some(true)
}

fn clean_xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(*c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}'))
        .collect()
}

fn node_text(node: &Node) -> String {
    if node.kind != "TEXT" {
        return String::new();
    }
    let clean = clean_xml(
        node.data
            .path(&["textData", "characters"])
            .and_then(Val::as_str)
            .unwrap_or(""),
    );
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
