use crate::html_changes::{PageLines, page_lines};
use crate::html_page::{is_shown, page_section, shared_only};
use crate::html_pictures::effective;
use crate::{DiffReport, FileMeta, PageDiff};
use std::fmt::Write;

pub(crate) fn escape(text: &str) -> String {
    html_escape::encode_quoted_attribute(text).into_owned()
}

pub(crate) fn page_name(page: &PageDiff) -> &str {
    page.new_name
        .as_deref()
        .or(page.old_name.as_deref())
        .unwrap_or("(unnamed)")
}

fn file_card(out: &mut String, label: &str, file: &FileMeta) {
    let _ = write!(
        out,
        "<div class=\"file\"><b>{label}</b><span class=\"fn\">{}</span><span class=\"fm\">{} pages · {} layers</span></div>",
        escape(&file.path_label),
        file.page_count,
        file.node_count
    );
}

fn hidden_pages(out: &mut String, hidden: &[&PageDiff]) {
    if hidden.is_empty() {
        return;
    }
    let _ = write!(
        out,
        "<details><summary>{} pages with no visible change</summary><ul>",
        hidden.len()
    );
    for page in hidden {
        let _ = write!(out, "<li>{}</li>", escape(page_name(page)));
    }
    out.push_str("</ul></details>");
}

fn header(out: &mut String, report: &DiffReport, hidden: &[&PageDiff]) {
    out.push_str("<header class=\"top\"><p class=\"brand\">figstudio · Version comparison</p>");
    let title = report
        .new
        .path_label
        .strip_suffix(".fig")
        .unwrap_or(&report.new.path_label);
    let _ = write!(
        out,
        "<h1>{}</h1><div class=\"files\">",
        escape(if title.is_empty() {
            "Version comparison"
        } else {
            title
        })
    );
    file_card(out, "BEFORE", &report.old);
    out.push_str("<span class=\"arrow\">→</span>");
    file_card(out, "AFTER", &report.new);
    out.push_str("</div><div class=\"stats\">");
    let s = &report.summary;
    for (label, added, removed, changed) in [
        ("Pages", s.pages_added, s.pages_removed, s.pages_changed),
        ("Layers", s.nodes_added, s.nodes_removed, s.nodes_changed),
    ] {
        let _ = write!(
            out,
            "<div class=\"stat\"><b>{label}</b><span class=\"a\">+{added}</span><span class=\"d\">−{removed}</span><span class=\"m\">~{changed}</span></div>"
        );
    }
    let coverage = if report.complete {
        "complete"
    } else {
        "incomplete"
    };
    let _ = write!(
        out,
        "<div class=\"stat\"><b>Moved</b><span>{}</span></div><div class=\"stat\"><b>Coverage</b><span>{coverage}</span></div></div><div class=\"notes\">",
        s.nodes_moved
    );
    hidden_pages(out, hidden);
    let moved: usize = report.pages.iter().map(|p| p.screens_moved).sum();
    if moved > 0 {
        let _ = write!(
            out,
            "<div class=\"warn\">{moved} screens moved or re-parented without change (hidden)</div>"
        );
    }
    if !report.truncated.is_empty() {
        let _ = write!(
            out,
            "<details><summary>{} report notes</summary><ul>",
            report.truncated.len()
        );
        for note in &report.truncated {
            let _ = write!(out, "<li>{}</li>", escape(note));
        }
        out.push_str("</ul></details>");
    }
    out.push_str("</div></header>");
}

fn index(out: &mut String, shown: &[(usize, &PageDiff, PageLines)], hidden: &[&PageDiff]) {
    let pixels = |p: &PageDiff| p.render.as_ref().and_then(effective).unwrap_or(0);
    let max = shown.iter().map(|(_, p, _)| pixels(p)).max().unwrap_or(0);
    let _ = write!(
        out,
        "<nav class=\"side\" aria-label=\"Pages\"><h2>PAGES · {} CHANGED</h2><ul class=\"idx\">",
        shown.len()
    );
    for (n, page, lines) in shown {
        let count = if shared_only(page, lines) {
            "shared only".to_owned()
        } else {
            format!(
                "{} screens changed",
                page.screens
                    .iter()
                    .filter(|s| s.status == crate::ScreenStatus::Changed)
                    .count()
            )
        };
        let px = pixels(page);
        let width = if max == 0 {
            0.0
        } else {
            100.0 * (px as f64 + 1.0).ln() / (max as f64 + 1.0).ln()
        };
        let _ = write!(
            out,
            "<li><a href=\"#page-{n}\"><span class=\"dot {}\"></span><span class=\"nm\">{}</span><span class=\"mt\"><span>{px} px</span><span>{count}</span><span class=\"bar\" style=\"--w:{width:.0}%\"></span></span></a></li>",
            format!("{:?}", page.status).to_lowercase(),
            escape(page_name(page))
        );
    }
    out.push_str("</ul>");
    hidden_pages(out, hidden);
    out.push_str("</nav>");
}

/// Produces a self-contained, script-free HTML report from the canonical diff.
#[must_use]
pub fn render_html(report: &DiffReport) -> String {
    let mut out = format!(
        concat!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
            "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src 'self'; style-src 'unsafe-inline'\">",
            "<title>Version comparison</title><style>{}</style></head><body>",
            "<input class=\"mr\" type=\"radio\" name=\"view\" id=\"view-side\" checked aria-label=\"Side by side\">",
            "<input class=\"mr\" type=\"radio\" name=\"view\" id=\"view-overlay\" aria-label=\"Overlay\">",
            "<div class=\"tool\"><span>View</span><span class=\"seg\"><label class=\"ls\" for=\"view-side\">Side by side</label><label class=\"lh\" for=\"view-overlay\">Overlay</label></span></div>",
            "<div class=\"shell\">"
        ),
        include_str!("html.css")
    );
    let (shown, hidden): (Vec<_>, Vec<_>) = report
        .pages
        .iter()
        .enumerate()
        .map(|(n, p)| (n, p, page_lines(p)))
        .partition(|(_, p, l)| is_shown(p, l));
    let hidden: Vec<_> = hidden.iter().map(|(_, p, _)| *p).collect();
    header(&mut out, report, &hidden);
    index(&mut out, &shown, &hidden);
    out.push_str("<main>");
    crate::html_shared::shared_section(&mut out, report);
    for (n, page, lines) in &shown {
        page_section(&mut out, *n, page, lines, &report.shared_changes);
    }
    out.push_str("<footer class=\"foot\">Full machine record: report.json</footer></main></div></body></html>");
    out
}
