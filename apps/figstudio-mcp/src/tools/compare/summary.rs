//! Bounded inline summary JSON and artifact-directory naming for the compare
//! tools. Kept beside `compare.rs` to keep both under ~300 lines.

use super::MAX_INLINE_PAGES;
use figstudio_diff::{DiffReport, PageDiff, PageStatus};
use figstudio_fig::{Document, Guid};
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::tools::design::Assets;

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

/// One page's full `PageDiff` JSON, or `None` when the page is unchanged or
/// unknown to the comparison. The `id` becomes the same `session:local`
/// string the summarize pages list and `compare_page`'s pageId input use.
pub fn page_json(old: &Document, new: &Document, id: Guid) -> Option<Value> {
    let report = figstudio_diff::diff_documents(old, new);
    let mut value = report
        .pages
        .iter()
        .find(|page| page.id == id)
        .and_then(|page| serde_json::to_value(page).ok())?;
    stringify_guids(&mut value);
    Some(value)
}

/// MCP compare output only: every Guid as `session:local`. Disk `report.json`
/// keeps the crate's native `[u32,u32]` shape.
fn stringify_guids(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if matches!(key.as_str(), "id" | "old_parent" | "new_parent")
                    && let Some(s) = guid_string(child)
                {
                    *child = Value::String(s);
                    continue;
                }
                stringify_guids(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                stringify_guids(item);
            }
        }
        _ => {}
    }
}

fn guid_string(value: &Value) -> Option<String> {
    let pair = value.as_array()?;
    if pair.len() != 2 {
        return None;
    }
    Some(format!("{}:{}", pair[0].as_u64()?, pair[1].as_u64()?))
}

/// A fresh, request-isolated report directory under the compare root.
///
/// The unique `compare-…` name means a symlink planted at any predictable
/// path (including the old `<old>-vs-<new>` name) is never followed, and two
/// requests — even for the same pair — never share a directory. The dir is
/// kept (`keep()`), not deleted on drop, because the tool returns its paths
/// and the client reads the artifacts after the response.
pub fn artifact_dir(assets: &Assets) -> std::io::Result<PathBuf> {
    let root = match assets {
        Assets::Dir(dir) => dir.join("compare"),
        Assets::Inline => std::env::temp_dir().join("figstudio-compare"),
    };
    std::fs::create_dir_all(&root)?;
    Ok(tempfile::Builder::new()
        .prefix("compare-")
        .tempdir_in(&root)?
        .keep())
}
