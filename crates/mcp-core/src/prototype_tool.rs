//! `get_prototype`: flows, the screens they reach and the links on each, as JSON.
use crate::{
    node_id::{NodeId, parse_node_id},
    output::Output,
};
use figstudio_fig::Document;
use figstudio_render::Renderer;
use figstudio_render::prototype::{self, Screen};
use serde_json::{Value, json};

/// Screens listed when the call gives no limit.
pub const DEFAULT_LIMIT: usize = 50;
/// The most screens a call may ask for.
pub const MAX_LIMIT: usize = 500;

/// The file's flows and, from `node` (a screen frame) or else from every flow (else the first
/// page's first frame), the screens reachable by played links, at most `limit`.
pub fn prototype(doc: &Document, node: Option<&str>, limit: Option<usize>) -> Output {
    let flows = prototype::flows(doc);
    let starts = match node.map(parse_node_id) {
        Some(Ok(NodeId::Node(id))) if doc.node(id).is_some() => vec![id],
        Some(Ok(NodeId::Node(id))) => {
            return Output::fault(format!("node {id} does not exist in this file"));
        }
        Some(Ok(NodeId::InstancePath(_))) => {
            return Output::fault("nodeId must be a frame, not a node inside an instance");
        }
        Some(Err(error)) => return Output::fault(error.to_string()),
        None if flows.is_empty() => doc
            .pages
            .first()
            .and_then(|page| prototype::start(doc, *page, None))
            .into_iter()
            .collect(),
        None => flows.iter().map(|f| f.start).collect(),
    };
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let (screens, truncated) = prototype::reachable(&mut renderer, doc, &starts, limit);
    let not_played: usize = screens.iter().map(Screen::skipped).sum();
    Output::structured(json!({
        "flows": flows.iter().map(|f| json!({
            "name": f.name,
            "description": f.description,
            "page": f.page.to_string(),
            "startNodeId": f.start.to_string(),
        })).collect::<Vec<_>>(),
        "screens": screens.iter().map(|s| screen(doc, s)).collect::<Vec<_>>(),
        "truncated": truncated,
        "notPlayed": not_played,
    }))
}

fn screen(doc: &Document, screen: &Screen) -> Value {
    json!({
        "nodeId": screen.frame.to_string(),
        "name": doc.node(screen.frame).map_or("", |n| n.name.as_str()),
        "links": screen.links.iter().map(|l| json!({
            "nodeId": l.layer.to_string(),
            "name": l.name,
            "trigger": l.trigger,
            "action": l.action,
            "destinationId": l.destination.map(|d| d.to_string()),
            "destinationMissing": l.destination.is_some_and(|d| doc.node(d).is_none()),
            "url": l.url,
            "transition": l.transition,
            "played": l.step.is_some(),
            "rect": [l.bounds.x0, l.bounds.y0, l.bounds.width(), l.bounds.height()],
        })).collect::<Vec<_>>(),
        "truncated": screen.truncated,
    })
}
