use std::fmt::Write;

use figstudio_fig::Val;

use crate::{DiffReport, NodeDiff, NodeRef, PropertyChange, RenderDiff};

fn escape(text: &str) -> String {
    html_escape::encode_quoted_attribute(text).into_owned()
}

/// Produces a self-contained, script-free HTML report from the canonical diff.
#[must_use]
pub fn render_html(report: &DiffReport) -> String {
    let mut out = String::from(concat!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\">",
        "<title>Version comparison</title><style>",
        "body{font-family:system-ui,sans-serif;max-width:72rem;margin:2rem auto;padding:0 1rem;color:#222}",
        "section{border-top:1px solid #aaa;margin-top:2rem}li{margin:.6rem 0;overflow-wrap:anywhere}",
        "pre{white-space:pre-wrap}small{display:block;color:#555}",
        "</style></head><body><header><h1>Version comparison</h1>"
    ));
    let _ = write!(
        out,
        "<p>{} → {}</p>",
        escape(&report.old.path_label),
        escape(&report.new.path_label)
    );
    let s = &report.summary;
    let _ = write!(
        out,
        "<p>Pages: {} added, {} removed, {} changed</p><p>Nodes: {} added, {} removed, {} changed, {} moved</p><p>Properties: {}</p>",
        s.pages_added,
        s.pages_removed,
        s.pages_changed,
        s.nodes_added,
        s.nodes_removed,
        s.nodes_changed,
        s.nodes_moved,
        s.properties_changed
    );
    let coverage = if report.complete {
        "complete"
    } else {
        "incomplete"
    };
    let _ = write!(out, "<p>Stored-change coverage: {coverage}</p><ul>");
    for note in &report.truncated {
        let _ = write!(out, "<li>{}</li>", escape(note));
    }
    out.push_str("</ul></header><main>");
    for page in &report.pages {
        let name = page
            .new_name
            .as_deref()
            .or(page.old_name.as_deref())
            .unwrap_or("(unnamed)");
        let _ = write!(
            out,
            "<section><h2>{}</h2><p>{:?}</p>",
            escape(name),
            page.status
        );
        if page.old_name != page.new_name {
            let _ = write!(
                out,
                "<p>Page name: {} → {}</p>",
                escape(page.old_name.as_deref().unwrap_or("(absent)")),
                escape(page.new_name.as_deref().unwrap_or("(absent)"))
            );
        }
        if let Some(render) = &page.render {
            render_links(&mut out, render);
            render_measures(&mut out, render);
        }
        if !page.moved_out.is_empty() {
            let ids: Vec<_> = page.moved_out.iter().map(ToString::to_string).collect();
            let _ = write!(out, "<p>Moved out: {}</p>", ids.join(", "));
        }
        references(&mut out, "Added", &page.added);
        references(&mut out, "Removed", &page.removed);
        out.push_str("<h3>Changed</h3><ul>");
        for change in &page.changed {
            changed(&mut out, change);
        }
        out.push_str("</ul></section>");
    }
    out.push_str("</main></body></html>");
    out
}

fn render_links(out: &mut String, render: &RenderDiff) {
    for (label, path) in [
        ("Before PNG", &render.old_png),
        ("After PNG", &render.new_png),
        ("Diff PNG", &render.diff_png),
    ] {
        if let Some(path) = path.as_deref().filter(|path| relative_link(path)) {
            // A fixed ./ prefix prevents an input scheme from becoming an active URL.
            let _ = write!(out, "<p><a href=\"./{}\">{label}</a></p>", escape(path));
        }
    }
}

fn render_measures(out: &mut String, render: &RenderDiff) {
    let mut parts = Vec::new();
    if let Some(pixels) = render.pixels_changed {
        parts.push(format!("{pixels} pixels changed"));
    }
    if let Some(worst) = render.worst_channel_delta {
        parts.push(format!("worst channel delta {worst}"));
    }
    if let (Some(width), Some(height)) = (render.width, render.height) {
        parts.push(format!("{width}x{height}"));
    }
    if !parts.is_empty() {
        let _ = writeln!(out, "<p>{}</p>", parts.join(", "));
    }
}

fn relative_link(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains([':', '\\', '?', '#', '%'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, ".." | ""))
}

fn references(out: &mut String, heading: &str, nodes: &[NodeRef]) {
    let _ = write!(out, "<h3>{heading}</h3><ul>");
    for node in nodes {
        let _ = write!(
            out,
            "<li>{} ({}) — {} (subtree size incl. movers)<small>{}</small></li>",
            escape(&node.name),
            escape(&node.kind),
            node.descendants,
            escape(&node.path.join(" / "))
        );
    }
    out.push_str("</ul>");
}

fn changed(out: &mut String, node: &NodeDiff) {
    let _ = write!(
        out,
        "<li>{} ({})<small>{}</small>",
        escape(&node.name),
        escape(&node.kind),
        escape(&node.path.join(" / "))
    );
    if let Some(moved) = &node.moved {
        let parent =
            |id: Option<figstudio_fig::Guid>| id.map_or_else(|| "(root)".into(), |v| v.to_string());
        let _ = write!(
            out,
            "<p>Moved: {} at {} → {} at {}</p>",
            parent(moved.old_parent),
            escape(&moved.old_position),
            parent(moved.new_parent),
            escape(&moved.new_position)
        );
    }
    properties(
        out,
        node.changes.iter().filter(|change| {
            node.text.as_ref().is_none_or(|text| {
                !text
                    .runs
                    .iter()
                    .any(|run| run.changes.iter().any(|c| c.field == change.field))
            })
        }),
    );
    if let Some(text) = &node.text {
        let _ = write!(
            out,
            "<pre>Text: {} → {}</pre>",
            escape(&text.old),
            escape(&text.new)
        );
        for run in &text.runs {
            let _ = write!(out, "<p>Style run {}</p>", run.index);
            properties(out, &run.changes);
        }
    }
    out.push_str("</li>");
}

fn properties<'a>(out: &mut String, changes: impl IntoIterator<Item = &'a PropertyChange>) {
    out.push_str("<ul>");
    for change in changes {
        let _ = write!(
            out,
            "<li>{}: {} → {}</li>",
            escape(&change.field),
            value(change.old.as_ref()),
            value(change.new.as_ref())
        );
    }
    out.push_str("</ul>");
}

fn value(value: Option<&Val>) -> String {
    value.map_or_else(
        || "(absent)".into(),
        |v| escape(&serde_json::to_string(v).expect("nonfinite floats are serialized as bits")),
    )
}
