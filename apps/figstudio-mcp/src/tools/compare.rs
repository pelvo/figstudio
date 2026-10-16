//! Native-only compare tools: the renderer must stay out of wasm mcp-core.
//! The browser schema test records both tools as explained omissions.

mod artifacts;
mod summary;

use crate::{
    FigstudioServer,
    error::McpError,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use artifacts::artifact_dir;
use figstudio_diff::{
    DiffReport, RenderImages, RenderOpts, attach_render_diffs_with_sink, diff_documents,
    render_html,
};
use figstudio_fig::{Document, Guid};
use figstudio_render::images::ImageRequest;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;
use summary::{page_json, summarize};
#[cfg(test)]
use summary::stringify_guids;

/// Page entries returned inline by `compare_files`; the rest stay in `report.json`.
pub const MAX_INLINE_PAGES: usize = 100;

const OLD_ERROR: &str = "cannot read the older file";
const NEW_ERROR: &str = "cannot read the newer file";
const WRITE_ERROR: &str = "cannot write the comparison report";
const SERIALIZE_ERROR: &str = "cannot serialize the comparison report";
const PAGE_NOT_FOUND: &str = "page not found in the comparison";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CompareFilesInput {
    /// Registered file key (stem or full path) of the older file.
    old_file: Option<String>,
    /// Registered file key (stem or full path) of the newer file.
    new_file: Option<String>,
    /// Render changed pages and write before/after/diff PNGs (default false).
    include_render: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComparePageInput {
    /// Registered file key (stem or full path) of the older file.
    old_file: Option<String>,
    /// Registered file key (stem or full path) of the newer file.
    new_file: Option<String>,
    /// Page GUID as `session:local` or `session-local`.
    page_id: String,
}

#[tool_router(router = compare_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Compare two registered local files: writes report.json and report.html (plus before/after/diff PNGs when includeRender is true) under the server assets directory, and returns counts, artifact paths and at most 100 page entries (id, status, added/removed/changed counts). Use compare_page for bounded single-page detail. Rendering is off by default and capped by the diff crate."
    )]
    async fn compare_files(
        &self,
        Parameters(input): Parameters<CompareFilesInput>,
    ) -> CallToolResult {
        let (old, _, old_path) = match self.files().open_picked(input.old_file.as_deref()).await {
            Ok(picked) => picked,
            Err(error) => return load_error(error, OLD_ERROR),
        };
        let (new, _, new_path) = match self.files().open_picked(input.new_file.as_deref()).await {
            Ok(picked) => picked,
            Err(error) => return load_error(error, NEW_ERROR),
        };
        let include_render = input.include_render.unwrap_or(false);
        let system_fonts = !self.no_system_fonts();
        // Rendering resolves by the picked paths (inside each `Rendering`),
        // never by re-looking-up a stem, so duplicate stems stay unambiguous.
        let (old_rendering, new_rendering) = if include_render {
            let old_rendering = match self.files().rendering_picked(&old_path, &old, system_fonts) {
                Ok(rendering) => rendering,
                Err(error) => return load_error(error, OLD_ERROR),
            };
            let new_rendering = match self.files().rendering_picked(&new_path, &new, system_fonts) {
                Ok(rendering) => rendering,
                Err(error) => return load_error(error, NEW_ERROR),
            };
            (Some(old_rendering), Some(new_rendering))
        } else {
            (None, None)
        };
        let dir = match artifact_dir(self.assets()) {
            Ok(dir) => dir,
            Err(_) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    "artifact directory unavailable",
                )]);
            }
        };
        let old_label = file_label(&old_path);
        let new_label = file_label(&new_path);
        let run = tokio::task::spawn_blocking(move || {
            compare_and_write(
                &old,
                &new,
                &old_label,
                &new_label,
                &dir,
                include_render,
                system_fonts,
                old_rendering.as_ref(),
                new_rendering.as_ref(),
            )
        })
        .await;
        match run {
            Ok(Ok(summary)) => CallToolResult::structured(summary),
            Ok(Err(message)) => {
                CallToolResult::error(vec![rmcp::model::ContentBlock::text(message)])
            }
            Err(error) => tool_error(McpError::load_task(error)),
        }
    }

    #[tool(
        description = "Return one page's node-level diff for two registered local files, re-running the comparison. Inline JSON is capped at 64 KiB by omitting whole entries, with per-list Total/Omitted counts and a truncated boolean. pageId is a page GUID such as 0:1. The report is not cached; use the report.json from compare_files for bulk detail."
    )]
    async fn compare_page(
        &self,
        Parameters(input): Parameters<ComparePageInput>,
    ) -> CallToolResult {
        let id = match parse_node_id(&input.page_id) {
            Ok(NodeId::Node(id)) => id,
            Ok(NodeId::InstancePath(_)) => {
                return CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    "nodeId is invalid",
                )]);
            }
            Err(error) => return tool_error(error.into()),
        };
        let (old, _, _) = match self.files().open_picked(input.old_file.as_deref()).await {
            Ok(picked) => picked,
            Err(error) => return load_error(error, OLD_ERROR),
        };
        let (new, _, _) = match self.files().open_picked(input.new_file.as_deref()).await {
            Ok(picked) => picked,
            Err(error) => return load_error(error, NEW_ERROR),
        };
        let run = tokio::task::spawn_blocking(move || page_json(&old, &new, id)).await;
        match run {
            Ok(Some(page)) => CallToolResult::structured(page),
            Ok(None) => {
                CallToolResult::error(vec![rmcp::model::ContentBlock::text(PAGE_NOT_FOUND)])
            }
            Err(error) => tool_error(McpError::load_task(error)),
        }
    }
}

/// Fixed text for a document load failure; other errors pass through because
/// they are already fixed strings (file-key and parse errors echo no input).
fn load_error(error: McpError, fixed: &'static str) -> CallToolResult {
    if error.is_open() {
        CallToolResult::error(vec![rmcp::model::ContentBlock::text(fixed)])
    } else {
        tool_error(error)
    }
}

/// Write reports and summarize; errors never echo file content or paths.
#[expect(clippy::too_many_arguments)]
fn compare_and_write(
    old: &Document,
    new: &Document,
    old_label: &str,
    new_label: &str,
    dir: &Path,
    include_render: bool,
    system_fonts: bool,
    old_rendering: Option<&crate::tools::rendering::Rendering>,
    new_rendering: Option<&crate::tools::rendering::Rendering>,
) -> Result<Value, &'static str> {
    let mut report = diff_documents(old, new);
    report.old.path_label = old_label.to_owned();
    report.new.path_label = new_label.to_owned();
    if include_render {
        render_pages(
            &mut report,
            old,
            new,
            dir,
            system_fonts,
            old_rendering,
            new_rendering,
        )?;
    }
    std::fs::create_dir_all(dir).map_err(|_| WRITE_ERROR)?;
    let json = report.to_json().map_err(|_| SERIALIZE_ERROR)?;
    std::fs::write(dir.join("report.json"), json).map_err(|_| WRITE_ERROR)?;
    std::fs::write(dir.join("report.html"), render_html(&report)).map_err(|_| WRITE_ERROR)?;
    Ok(summarize(
        &report,
        &dir.join("report.json").display().to_string(),
        &dir.join("report.html").display().to_string(),
    ))
}

/// Render changed pages like the CLI does: stream each page's PNGs to disk,
/// then link the written files from the report.
fn render_pages(
    report: &mut DiffReport,
    old: &Document,
    new: &Document,
    dir: &Path,
    system_fonts: bool,
    old_rendering: Option<&crate::tools::rendering::Rendering>,
    new_rendering: Option<&crate::tools::rendering::Rendering>,
) -> Result<(), &'static str> {
    let render_dir = dir.join("render");
    std::fs::create_dir_all(&render_dir).map_err(|_| WRITE_ERROR)?;
    let empty = crate::tools::rendering::Rendering::default();
    let (old_rendering, new_rendering) = (
        old_rendering.unwrap_or(&empty),
        new_rendering.unwrap_or(&empty),
    );
    let old_read = |want: &ImageRequest| old_rendering.read(&want.entry);
    let new_read = |want: &ImageRequest| new_rendering.read(&want.entry);
    let opts = RenderOpts {
        scale: 1.0,
        system_fonts,
        ..RenderOpts::default()
    };
    let mut written: Vec<(Guid, [Option<String>; 3])> = Vec::new();
    let mut failed = false;
    attach_render_diffs_with_sink(
        old,
        new,
        report,
        &opts,
        &old_read,
        &new_read,
        &mut |id, images| {
            if failed {
                return;
            }
            match write_page(dir, id, images) {
                Ok(names) => written.push((id, names)),
                Err(()) => failed = true,
            }
        },
    );
    if failed {
        return Err(WRITE_ERROR);
    }
    for (id, [before, after, diff]) in written {
        if let Some(render) = report
            .pages
            .iter_mut()
            .find(|page| page.id == id)
            .and_then(|page| page.render.as_mut())
        {
            render.old_png = before;
            render.new_png = after;
            render.diff_png = diff;
        }
    }
    Ok(())
}

/// Writes PNGs under `dir/render/`, named only by GUID u32s.
fn write_page(dir: &Path, id: Guid, images: &RenderImages) -> Result<[Option<String>; 3], ()> {
    let base = format!("render/{}-{}-", id.0, id.1);
    let put = |suffix: &str, bytes: &Option<Vec<u8>>| -> Result<Option<String>, ()> {
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let name = format!("{base}{suffix}.png");
        std::fs::write(dir.join(&name), bytes).map_err(|_| ())?;
        Ok(Some(name))
    };
    Ok([
        put("before", &images.before)?,
        put("after", &images.after)?,
        put("diff", &images.diff)?,
    ])
}

fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || "(unnamed)".into(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod tests;
