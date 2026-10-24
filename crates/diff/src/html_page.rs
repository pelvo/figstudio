//! Page headers, screen blocks, non-screen changes and folded whole-page context.
use crate::html::{escape, page_name, plural};
use crate::html_changes::PageLines;
use crate::html_lines::render_lines;
use crate::html_pictures::{measures, whole_page};
use crate::html_screen::{only_shared, screen_section};
use crate::{PageDiff, ScreenStatus, SharedChange};
use std::fmt::Write;

pub(crate) fn is_shown(page: &PageDiff, lines: &PageLines) -> bool {
    !page.screens.is_empty()
        || lines.items.iter().any(|l| {
            page.other_nodes
                .as_ref()
                .is_none_or(|ids| ids.contains(&l.id))
        })
}

pub(crate) fn shared_only(page: &PageDiff, lines: &PageLines) -> bool {
    !page.screens.is_empty()
        && page.screens.iter().all(|s| only_shared(s, lines))
        && !lines.items.iter().any(|l| {
            page.other_nodes
                .as_ref()
                .is_none_or(|ids| ids.contains(&l.id))
        })
}

pub(crate) fn page_section(
    out: &mut String,
    n: usize,
    page: &PageDiff,
    lines: &PageLines,
    shared: &[SharedChange],
) {
    let status = format!("{:?}", page.status);
    let _ = write!(
        out,
        "<section class=\"page\" id=\"page-{n}\"><header class=\"ph\"><h2>{}</h2><span class=\"badge {}\">{status}</span>",
        escape(page_name(page)),
        status.to_lowercase()
    );
    if page.old_name != page.new_name {
        let _ = write!(
            out,
            "<div class=\"rn\"><span class=\"o\">{}</span><span class=\"ar\">→</span><span class=\"n\">{}</span></div>",
            escape(page.old_name.as_deref().unwrap_or("(absent)")),
            escape(page.new_name.as_deref().unwrap_or("(absent)"))
        );
    }
    let count = |status| page.screens.iter().filter(|s| s.status == status).count();
    let _ = write!(
        out,
        "<div class=\"meta\"><span>Screens: {} changed · {} added · {} removed</span>",
        count(ScreenStatus::Changed),
        count(ScreenStatus::Added),
        count(ScreenStatus::Removed)
    );
    if let Some(render) = &page.render {
        let _ = write!(out, "<span>{}</span>", measures(render));
    }
    out.push_str("</div></header>");
    for (k, screen) in page.screens.iter().enumerate() {
        if !only_shared(screen, lines) {
            screen_section(out, n, k + 1, screen, lines, shared);
        }
    }
    crate::html_shared::folded_screens(out, n, page, lines);
    if page.screens_moved > 0 {
        let _ = write!(
            out,
            "<p class=\"none\">{} moved or re-parented without change (hidden)</p>",
            plural(page.screens_moved, "screen", "screens")
        );
    }
    let rest = || {
        lines.items.iter().filter(|l| {
            page.other_nodes
                .as_ref()
                .is_none_or(|ids| ids.contains(&l.id))
        })
    };
    // Only when there is something to list: an empty "None." section is noise.
    if rest().next().is_some() {
        out.push_str(
            "<div class=\"sec\"><div class=\"sh\"><h3>Other changes on this page</h3></div>",
        );
        render_lines(out, rest());
        out.push_str("</div>");
    }
    if let Some(render) = &page.render {
        whole_page(out, render);
    }
    if lines.other_props > 0 {
        let _ = write!(
            out,
            "<p class=\"foot\">{} on {} {} only in report.json</p>",
            plural(
                lines.other_props,
                "other property change",
                "other property changes"
            ),
            plural(lines.other_layers, "layer", "layers"),
            if lines.other_props == 1 { "is" } else { "are" }
        );
    }
    out.push_str("</section>");
}
