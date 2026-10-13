//! `get_nodes_in_region`: the nodes of a page that a rectangle touches, as JSON.
use crate::{
    error::CoreError,
    link::{parse_link, validate_rect},
    node_id::parse_guid,
    output::Output,
    region::{DEFAULT_BUDGET, DEFAULT_LIMIT, nodes_in},
    region_shot::BAD_RECT,
};
use figstudio_fig::{Document, Guid};
use serde_json::json;

/// The most rows a call may ask for.
pub const MAX_LIMIT: usize = 500;

/// A region link, or `page` + `rect`, as the page and rect it names.
///
/// # Errors
/// A message for a tool error: both forms or neither, a link that is no region link, a bad
/// page ID, or a bad rect.
pub fn resolve(
    region: Option<&str>,
    page: Option<&str>,
    rect: Option<[f64; 4]>,
) -> Result<(Guid, [f64; 4]), CoreError> {
    match (region, page, rect) {
        (Some(link), None, None) => match parse_link(link) {
            Ok(l) => l.page.zip(l.rect).ok_or_else(not_a_region),
            Err(_) => Err(not_a_region()),
        },
        (None, Some(page), Some(rect)) => {
            let page = parse_guid(page)
                .map_err(|_| CoreError::input(format!("invalid page ID {page:?}")))?;
            Ok((
                page,
                validate_rect(rect).map_err(|_| CoreError::input(BAD_RECT))?,
            ))
        }
        _ => Err(CoreError::input(
            "give either region (a figstudio region link) or page and rect",
        )),
    }
}

fn not_a_region() -> CoreError {
    CoreError::input("region must be a figstudio://<fileKey>?page=<id>&rect=x,y,w,h link")
}

/// The nodes of the region as `{page, rect, nodes: [{nodeId, name, kind}], total, truncated}`,
/// top-level first, at most `limit` (default 50, at most [`MAX_LIMIT`]).
pub fn nodes_in_region(
    doc: &Document,
    region: Option<&str>,
    page: Option<&str>,
    rect: Option<[f64; 4]>,
    limit: Option<usize>,
) -> Output {
    let (page, rect) = match resolve(region, page, rect) {
        Ok(found) => found,
        Err(error) => return Output::fault(error.to_string()),
    };
    if doc.node(page).is_none_or(|n| n.kind != "CANVAS") {
        return Output::fault(format!("page {page} does not exist in this file"));
    }
    let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let found = nodes_in(doc, page, rect, limit, DEFAULT_BUDGET);
    let nodes: Vec<_> = found
        .nodes
        .iter()
        .map(|n| json!({"nodeId": n.id.to_string(), "name": n.name, "kind": n.kind}))
        .collect();
    Output::structured(json!({
        "page": page.to_string(),
        "rect": rect,
        "nodes": nodes,
        "total": found.total,
        "truncated": found.truncated,
    }))
}
