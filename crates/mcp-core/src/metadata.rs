//! Sparse metadata XML for pages and local node subtrees.
use crate::{
    node_id::{NodeId, parse_node_id},
    output::{Block, Output},
};
use figjam::Extras;
use figstudio_fig::{
    Document, Guid, Node,
    resolve::{InstancePath, walk_path},
};
use node::{tag_name, write_node};
use quick_xml::{
    Writer,
    events::{BytesStart, Event},
};

pub(crate) mod figjam;
mod node;
mod pages;
mod resolved;
use resolved::Out;

/// Return a page list or bounded subtree as XML; invalid IDs include the page list.
pub fn metadata(doc: &Document, node_id: Option<&str>, depth: Option<u32>) -> Output {
    metadata_within(doc, node_id, depth, DEFAULT_OUTPUT_BYTES)
}

/// [`metadata`] with an explicit output byte budget (`--max-output-bytes`).
pub fn metadata_within(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    max_bytes: usize,
) -> Output {
    match run(doc, node_id, depth, None, max_bytes) {
        Ok((xml, _)) => Output::success(vec![Block::text(xml)]),
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
    max_bytes: usize,
) -> Result<(String, Option<Extras>), Output> {
    let root = match node_id {
        None => None,
        Some(input) => match parse_node_id(input) {
            Ok(NodeId::Node(id)) if doc.node(id).is_some() => Some(Root::Node(id)),
            Ok(NodeId::InstancePath(path)) => Some(Root::Path(path)),
            _ => return Err(unknown(doc)),
        },
    };
    match xml(doc, root, depth, extras, max_bytes) {
        Ok(Some(out)) => Ok(out),
        Ok(None) => Err(unknown(doc)),
        Err(error) => Err(Output::error(vec![Block::text(format!(
            "could not write metadata XML: {error}"
        ))])),
    }
}

fn unknown(doc: &Document) -> Output {
    let pages = xml(doc, None, None, None, DEFAULT_OUTPUT_BYTES)
        .map(|out| out.map(|(xml, _)| xml).unwrap_or_default())
        .unwrap_or_else(|error| error.to_string());
    Output::error(vec![Block::text(format!(
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
/// Default output bytes, closing tags included. Claude Code's inline limit is about 25k
/// tokens: a 20,166-char reply was delivered and a 25,810-char one too, while 44,674 and
/// 74,213 chars were rejected. The default stays under the lower of those measured
/// delivered sizes; `--max-output-bytes` raises or lowers it. A cut agent calls again with
/// a child nodeId or a smaller depth.
pub const DEFAULT_OUTPUT_BYTES: usize = 24_000;
/// Upper bound on one element's bytes (name and text are capped, so this is generous).
const ELEMENT_MAX_BYTES: usize = 1_024;

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
    max_bytes: usize,
) -> std::io::Result<Option<XmlOut>> {
    let mut writer = Writer::new(Vec::new());
    let mut figjam = figjam;
    match root {
        None => pages::write(doc, &mut writer, max_bytes)?,
        Some(root) => {
            let mut out = Out {
                body: Writer::new(Vec::new()),
                count: 0,
                truncated: false,
                close_reserve: 0,
                skipped: 0,
                definitions: None,
                figjam: figjam.take(),
                max_bytes,
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
            Step::End(kind) => {
                out.close_reserve -= kind.len() + 3;
                out.body
                    .write_event(Event::End(BytesStart::new(kind).to_end()))?
            }
            Step::Children(id, level, index) => {
                if let Some(child) = doc.children(id).get(index) {
                    if doc.node(*child).is_some_and(is_internal) {
                        stack.push(Step::Children(id, level, index + 1));
                        continue;
                    }
                    if out.count == MAX_ELEMENTS || !out.has_room() {
                        out.truncated = true;
                        continue;
                    }
                    stack.push(Step::Children(id, level, index + 1));
                    stack.push(Step::Visit(*child, level));
                }
            }
            Step::Visit(id, level) => {
                if out.count == MAX_ELEMENTS || !out.has_room() {
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
                    let tag = tag_name(&node.kind);
                    out.close_reserve += tag.len() + 3;
                    stack.push(Step::End(tag));
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
