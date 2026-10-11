//! Sparse metadata XML for pages and local node subtrees.
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use figjam::Extras;
use figstudio_fig::{
    Document, Guid, Node,
    resolve::{InstancePath, walk_path},
};
use node::{clean_xml, tag_name, write_node};
use quick_xml::{
    Writer,
    events::{BytesStart, Event},
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

pub(crate) mod figjam;
mod node;
mod resolved;
use resolved::Out;

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
        description = "Get sparse XML metadata: node IDs, names, types, parent-relative positions and sizes. Rotation is degrees, counter-clockwise positive (Figma convention). Call without nodeId for visible pages. Canvas/document elements omit geometry; unsized nodes omit dimensions. Instances list their component's children with overrides applied, with ids like I<instance>;<child> that every tool accepts; componentId names the main component. depth limits children. Output caps at 5000 nodes: truncated=\"true\" on <metadata> or <pages> marks omitted nodes. Text caps at 200 characters plus an ellipsis when cut. Use get_design_context for implementation details when available."
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
    match run(doc, node_id, depth, None) {
        Ok((xml, _)) => CallToolResult::success(vec![ContentBlock::text(xml)]),
        Err(result) => result,
    }
}

/// The XML for `node_id` (or the page list), with FigJam output when `extras` is given.
/// `Err` is the finished tool result for a bad id or a write fault.
pub(crate) fn run(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    extras: Option<Extras>,
) -> Result<(String, Option<Extras>), CallToolResult> {
    let root = match node_id {
        None => None,
        Some(input) => match parse_node_id(input) {
            Ok(NodeId::Node(id)) if doc.node(id).is_some() => Some(Root::Node(id)),
            Ok(NodeId::InstancePath(path)) => Some(Root::Path(path)),
            _ => return Err(unknown(doc)),
        },
    };
    match xml(doc, root, depth, extras) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err(unknown(doc)),
        Err(error) => Err(CallToolResult::error(vec![ContentBlock::text(format!(
            "could not write metadata XML: {error}"
        ))])),
    }
}

fn unknown(doc: &Document) -> CallToolResult {
    let pages = xml(doc, None, None, None)
        .map(|out| out.map(|(xml, _)| xml).unwrap_or_default())
        .unwrap_or_else(|error| error.to_string());
    CallToolResult::error(vec![ContentBlock::text(format!(
        "Unknown or invalid node ID. Available pages:\n{pages}"
    ))])
}

/// Where a subtree starts: a document node or a node inside an instance.
enum Root {
    Node(Guid),
    Path(InstancePath),
}

// Bounds node output and stack memory even for deeply nested or very wide documents.
const MAX_ELEMENTS: usize = 5_000;
const MAX_TEXT_CHARS: usize = 200;

enum Step {
    Visit(Guid, u32),
    Children(Guid, u32, usize),
    End(String),
}

type XmlOut = (String, Option<Extras>);

fn xml(
    doc: &Document,
    root: Option<Root>,
    depth: Option<u32>,
    figjam: Option<Extras>,
) -> std::io::Result<Option<XmlOut>> {
    let mut writer = Writer::new(Vec::new());
    let mut figjam = figjam;
    match root {
        None => pages(doc, &mut writer)?,
        Some(root) => {
            let mut out = Out {
                body: Writer::new(Vec::new()),
                count: 0,
                truncated: false,
                skipped: 0,
                definitions: None,
                figjam: figjam.take(),
            };
            let found = match root {
                Root::Node(id) => subtree(doc, id, depth, &mut out).map(|()| true)?,
                Root::Path(path) => path_subtree(doc, &path, depth, &mut out)?,
            };
            if !found {
                return Ok(None);
            }
            figjam = out.finish(&mut writer)?;
        }
    }
    String::from_utf8(writer.into_inner())
        .map(|xml| Some((xml, figjam)))
        .map_err(std::io::Error::other)
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

/// A node inside an instance and its resolved descendants; `false` when the path names none.
fn path_subtree(
    doc: &Document,
    path: &InstancePath,
    depth: Option<u32>,
    out: &mut Out,
) -> std::io::Result<bool> {
    let definitions = out.definitions(doc);
    let max_level = depth.map(|d| d as usize);
    // One more than the cap tells a full output apart from a truncated one.
    let Ok(walk) = walk_path(doc, &definitions, path, max_level, MAX_ELEMENTS + 1) else {
        return Ok(false);
    };
    out.truncated |= walk.truncated || walk.nodes.len() > MAX_ELEMENTS;
    out.skipped += walk.skipped_instances;
    out.resolved(doc, &walk.nodes[..walk.nodes.len().min(MAX_ELEMENTS)])?;
    Ok(true)
}

fn subtree(doc: &Document, root: Guid, depth: Option<u32>, out: &mut Out) -> std::io::Result<()> {
    let mut stack = vec![Step::Visit(root, 0)];
    while let Some(step) = stack.pop() {
        match step {
            Step::End(kind) => out
                .body
                .write_event(Event::End(BytesStart::new(kind).to_end()))?,
            Step::Children(id, level, index) => {
                if let Some(child) = doc.children(id).get(index) {
                    if doc.node(*child).is_some_and(is_internal) {
                        stack.push(Step::Children(id, level, index + 1));
                        continue;
                    }
                    if out.count == MAX_ELEMENTS {
                        out.truncated = true;
                        continue;
                    }
                    stack.push(Step::Children(id, level, index + 1));
                    stack.push(Step::Visit(*child, level));
                }
            }
            Step::Visit(id, level) => {
                if out.count == MAX_ELEMENTS {
                    out.truncated = true;
                    continue;
                }
                let Some(node) = doc.node(id) else {
                    continue;
                };
                if is_internal(node) {
                    continue;
                }
                out.count += 1;
                let deeper = depth.is_none_or(|limit| level < limit);
                let instance = node.kind == "INSTANCE";
                let inside = (instance && deeper)
                    .then(|| {
                        let max_level = depth.map(|limit| (limit - level) as usize);
                        out.walk(doc, id, max_level, MAX_ELEMENTS - out.count)
                    })
                    .flatten();
                let has_children = match &inside {
                    Some(walk) => !walk.nodes.is_empty(),
                    None => {
                        !instance
                            && deeper
                            && doc
                                .children(id)
                                .iter()
                                .filter_map(|id| doc.node(*id))
                                .any(|child| !is_internal(child))
                    }
                };
                let shown = id.to_string();
                if let Some(extras) = &mut out.figjam {
                    extras.note(&shown, &node.kind);
                }
                let open = write_node(
                    &mut out.body,
                    &shown,
                    &node.kind,
                    &node.data,
                    has_children,
                    out.figjam.is_some(),
                )?;
                if let Some(walk) = &inside {
                    out.resolved(doc, &walk.nodes)?;
                }
                if open {
                    stack.push(Step::End(tag_name(&node.kind)));
                }
                if has_children && inside.is_none() {
                    stack.push(Step::Children(id, level + 1, 0));
                }
            }
        }
    }
    Ok(())
}

fn is_internal(node: &Node) -> bool {
    node.kind == "CANVAS" && node.data.flag("internalOnly") == Some(true)
}
