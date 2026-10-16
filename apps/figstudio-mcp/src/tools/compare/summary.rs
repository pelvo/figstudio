//! Bounded inline summary JSON for the compare tools.

use super::MAX_INLINE_PAGES;
use figstudio_diff::{DiffReport, PageDiff, PageStatus};
use figstudio_fig::{Document, Guid};
use serde_json::{Value, json};

/// Bounded inline summary: counts plus at most [`MAX_INLINE_PAGES`] page
/// entries; the full detail stays in `report.json` on disk.
pub fn summarize(report: &DiffReport, report_json: &str, report_html: &str) -> Value {
    let total = report.pages.len();
    let pages: Vec<Value> = report
        .pages
        .iter()
        .take(MAX_INLINE_PAGES)
        .map(page_summary)
        .collect();
    json!({
        "old": {"label": report.old.path_label, "formatVersion": report.old.format_version,
                "pageCount": report.old.page_count, "nodeCount": report.old.node_count},
        "new": {"label": report.new.path_label, "formatVersion": report.new.format_version,
                "pageCount": report.new.page_count, "nodeCount": report.new.node_count},
        "summary": {
            "pagesAdded": report.summary.pages_added,
            "pagesRemoved": report.summary.pages_removed,
            "pagesChanged": report.summary.pages_changed,
            "nodesAdded": report.summary.nodes_added,
            "nodesRemoved": report.summary.nodes_removed,
            "nodesChanged": report.summary.nodes_changed,
            "nodesMoved": report.summary.nodes_moved,
            "propertiesChanged": report.summary.properties_changed,
        },
        "complete": report.complete,
        "truncated": report.truncated,
        "pages": pages,
        "pagesTotal": total,
        "pagesTruncated": total > MAX_INLINE_PAGES,
        "reportJson": report_json,
        "reportHtml": report_html,
    })
}

fn page_summary(page: &PageDiff) -> Value {
    let (name, name_truncated) = page
        .new_name
        .as_deref()
        .or(page.old_name.as_deref())
        .map_or((Value::Null, false), |name| {
            let (name, cut) = truncated(name);
            (Value::String(name), cut)
        });
    json!({
        "id": page.id.to_string(),
        "name": name,
        "nameTruncated": name_truncated,
        "status": status_label(page.status),
        "added": page.added.len(),
        "removed": page.removed.len(),
        "changed": page.changed.len(),
    })
}

fn status_label(status: PageStatus) -> &'static str {
    match status {
        PageStatus::Added => "added",
        PageStatus::Removed => "removed",
        PageStatus::Changed => "changed",
        PageStatus::Unchanged => "unchanged",
    }
}

/// Longest page name kept inline (bytes, on a char boundary); longer names
/// end in `~` and set `nameTruncated`. The disk report keeps the full name.
const MAX_NAME_BYTES: usize = 128;

fn truncated(name: &str) -> (String, bool) {
    if name.len() <= MAX_NAME_BYTES {
        return (name.to_owned(), false);
    }
    let mut end = MAX_NAME_BYTES - 1; // room for `~`
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}~", &name[..end]), true)
}

/// One page's bounded `PageDiff` JSON, or `None` for an unchanged/unknown page.
/// Whole entries are omitted above 64 KiB with totals and omitted counts.
/// The `id` becomes the same `session:local`
/// string the summarize pages list and `compare_page`'s pageId input use.
pub fn page_json(old: &Document, new: &Document, id: Guid) -> Option<Value> {
    let report = figstudio_diff::diff_documents(old, new);
    let page = report.pages.iter().find(|page| page.id == id)?;
    let mut value = serde_json::to_value(page).ok()?;
    value["status"] = status_label(page.status).into();
    stringify_guids(&mut value);
    bound_page(&mut value);
    Some(value)
}

/// Rewrite schema GUIDs only, never arbitrary stored property values.
pub(super) fn stringify_guids(value: &mut Value) {
    stringify_guid(&mut value["id"]);
    if let Some(moved_out) = value["moved_out"].as_array_mut() {
        for id in moved_out {
            stringify_guid(id);
        }
    }
    for field in ["added", "removed", "changed"] {
        let Some(nodes) = value[field].as_array_mut() else {
            continue;
        };
        for node in nodes {
            stringify_guid(&mut node["id"]);
            let Some(moved) = node.get_mut("moved").filter(|v| v.is_object()) else {
                continue;
            };
            stringify_guid(&mut moved["old_parent"]);
            stringify_guid(&mut moved["new_parent"]);
        }
    }
}

fn stringify_guid(value: &mut Value) {
    if let Some(id) = guid_string(value) {
        *value = Value::String(id);
    }
}

fn guid_string(value: &Value) -> Option<String> {
    let pair = value.as_array()?;
    if pair.len() != 2 {
        return None;
    }
    Some(format!("{}:{}", pair[0].as_u64()?, pair[1].as_u64()?))
}

/// Trim the largest whole entries first; disk artifacts remain full.
fn bound_page(value: &mut Value) {
    const LIMIT: usize = 65_536;
    let fields = ["added", "removed", "changed", "moved_out"];
    value["truncated"] = false.into();
    let mut lengths: Vec<Vec<usize>> = fields
        .iter()
        .map(|field| {
            let items = value[field].as_array().expect("page list");
            items
                .iter()
                .map(|item| serde_json::to_vec(item).expect("JSON value").len())
                .collect()
        })
        .collect();
    for (field, sizes) in fields.iter().zip(&lengths) {
        value[format!("{field}Total")] = sizes.len().into();
        value[format!("{field}Omitted")] = 0.into();
    }
    let mut bytes = serde_json::to_vec(value).expect("JSON value").len();
    if bytes <= LIMIT {
        return;
    }
    value["truncated"] = true.into();
    // Bound page metadata too: a name alone can exceed the inline budget.
    for field in ["old_name", "new_name"] {
        if let Some(name) = value[field].as_str() {
            let (name, cut) = truncated(name);
            value[field] = name.into();
            if cut {
                value["nameTruncated"] = true.into();
            }
        }
    }
    bytes = serde_json::to_vec(value).expect("JSON value").len();
    while bytes > LIMIT {
        let Some((index, pos, size)) = lengths
            .iter()
            .enumerate()
            .flat_map(|(index, sizes)| {
                sizes
                    .iter()
                    .enumerate()
                    .map(move |(pos, &size)| (index, pos, size))
            })
            .max_by_key(|&(_, _, size)| size)
        else {
            break;
        };
        lengths[index].remove(pos);
        let removed = size + usize::from(!lengths[index].is_empty());
        value[fields[index]]
            .as_array_mut()
            .expect("page list")
            .remove(pos);
        let key = format!("{}Omitted", fields[index]);
        let old = value[&key].as_u64().expect("omitted count");
        value[&key] = (old + 1).into();
        bytes = bytes - removed + (old + 1).to_string().len() - old.to_string().len();
    }
}
