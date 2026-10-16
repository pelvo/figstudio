use super::{McpTab, McpTabEnv};
use crate::{
    CoreError,
    link::{FileHint, parse_link},
    node_id::{NodeId, parse_node_id},
    output::Output,
};
use figstudio_fig::Guid;
use serde_json::{Value, json};

pub(super) fn pick<'a>(
    env: &'a dyn McpTabEnv,
    key: Option<&str>,
    link: Option<&str>,
) -> Result<&'a McpTab, CoreError> {
    let hint = link.and_then(|s| parse_link(s).ok()).and_then(|l| l.file);
    let key = key.map(str::to_owned).or_else(|| match hint {
        Some(FileHint::Exact(key)) => Some(key),
        Some(FileHint::Figma { key, slug }) => env
            .tabs()
            .iter()
            .find(|t| {
                t.file_key == key
                    || slug
                        .as_ref()
                        .is_some_and(|s| t.file_key.replace(' ', "-") == *s)
            })
            .map(|t| t.file_key.clone()),
        None => None,
    });
    let tab = match key {
        Some(key) => env.tabs().iter().find(|t| t.file_key == key),
        None => env
            .tabs()
            .iter()
            .find(|t| Some(t.id) == env.active_tab())
            .or_else(|| (env.tabs().len() == 1).then(|| &env.tabs()[0])),
    };
    tab.ok_or_else(|| CoreError::input("No matching open tab; use list_open_tabs"))
}

pub(super) fn target(tab: &McpTab, node: Option<&str>) -> Result<String, CoreError> {
    node.map(str::to_owned)
        .or_else(|| tab.selection.map(|id| id.to_string()))
        .ok_or_else(|| CoreError::input("nodeId is required when nothing is selected"))
}

pub(super) fn tabs(env: &dyn McpTabEnv) -> Output {
    Output::structured(json!({"tabs":env.tabs().iter().map(|t| json!({
        "tabId":t.id,"fileKey":t.file_key,"title":t.name,"page":page(t),
        "selection":t.selection.map(|id| id.to_string()),"viewport":t.viewport,
        "active":Some(t.id)==env.active_tab()
    })).collect::<Vec<_>>()}))
}

pub(super) fn selection(tab: &McpTab) -> Output {
    let node = tab.selection.and_then(|id| {
        let node = tab.doc.node(id)?;
        let bounds = figstudio_render::node_bounds(&tab.doc, id)
            .map(|r| json!({"x":r.x0,"y":r.y0,"width":r.width(),"height":r.height()}));
        Some(json!({"nodeId":id.to_string(),"name":node.name,"kind":node.kind,"bounds":bounds}))
    });
    Output::structured(json!({"fileKey":tab.file_key,"tabId":tab.id,"page":page(tab),"node":node}))
}

fn page(tab: &McpTab) -> Value {
    json!({"id":tab.page.to_string(),"name":tab.doc.node(tab.page).map(|n| &n.name)})
}

pub(super) fn reveal(env: &dyn McpTabEnv, tab: &McpTab, input: &str) -> Output {
    let result = (|| {
        let node: Guid = match parse_node_id(input)? {
            NodeId::Node(id) => id,
            NodeId::InstancePath(path) => path.instance(),
        };
        if tab.doc.node(node).is_none() {
            return Err(CoreError::input("Node is not in this file"));
        }
        env.reveal(tab.id, node)?;
        Ok(Output::success(vec![crate::output::Block::text(format!(
            "revealed {node} in tab {}",
            tab.id
        ))]))
    })();
    result.unwrap_or_else(|e: CoreError| Output::fault(e.to_string()))
}
