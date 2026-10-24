//! Shared-change cards reuse the same comparison component and CSS view controls.
use crate::html::{escape, page_name};
use crate::html_changes::PageLines;
use crate::html_pictures::compare_sized;
use crate::html_screen::only_shared;
use crate::{DiffReport, PageDiff};
use std::{collections::BTreeSet, fmt::Write};

pub(crate) fn shared_section(out: &mut String, report: &DiffReport) {
    if report.shared_changes.is_empty() {
        return;
    }
    out.push_str(
        "<section class=\"page shared\"><header class=\"ph\"><h2>Shared changes</h2></header>",
    );
    for group in &report.shared_changes {
        let k = group.id + 1;
        let pages: BTreeSet<_> = group.occurrences.iter().map(|o| o.page_index).collect();
        let _ = write!(
            out,
            "<div class=\"sec\" id=\"shared-{k}\"><div class=\"sh\">"
        );
        match group.component.as_ref().filter(|_| group.component_view) {
            Some(c) => {
                let title = c.set.as_ref().map_or(&c.name, |s| &s.name);
                let _ = write!(out, "<h3>Component {} changed</h3>", escape(title));
                if c.variants.len() > 1 {
                    let _ = write!(
                        out,
                        "<span class=\"none\">{} variants</span>",
                        c.variants.len()
                    );
                }
                let _ = write!(out, "<span class=\"tag\">Shared change #{k}</span>");
            }
            None => {
                let _ = write!(out, "<h3>Shared change #{k}</h3>");
            }
        }
        out.push_str("</div>");
        compare_sized(
            out,
            (&group.old_png, &group.new_png),
            group.image_width.zip(group.image_height),
            &format!("Shared change {k}"),
        );
        other_variants(out, group);
        let _ = write!(
            out,
            "<p class=\"none\">Appears in {} screens on {} pages</p><details><summary>Where this appears</summary><ul>",
            group.count,
            pages.len()
        );
        // The lead occurrence comes first in the data; the list reads in document order.
        let places: BTreeSet<_> = group
            .occurrences
            .iter()
            .map(|o| (o.page_index, o.screen_index))
            .collect();
        for (n, s) in places {
            if let Some(page) = report.pages.get(n)
                && let Some(screen) = page.screens.get(s)
            {
                let s = s + 1;
                let _ = write!(
                    out,
                    "<li><a href=\"#page-{n}\">{}</a> / <a href=\"#page-{n}-s{s}\">{}</a></li>",
                    escape(page_name(page)),
                    escape(&screen.name)
                );
            }
        }
        out.push_str("</ul></details></div>");
    }
    out.push_str("</section>");
}

pub(crate) fn folded_screens(out: &mut String, n: usize, page: &PageDiff, lines: &PageLines) {
    let folded: Vec<_> = page
        .screens
        .iter()
        .enumerate()
        .filter(|(_, s)| only_shared(s, lines))
        .collect();
    if folded.is_empty() {
        return;
    }
    let _ = write!(
        out,
        "<div class=\"sec\"><p class=\"none\">{} screens only show shared changes: ",
        folded.len()
    );
    for (i, (k, screen)) in folded.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let k = k + 1;
        let _ = write!(
            out,
            "<span id=\"page-{n}-s{k}\">{}</span>",
            escape(&screen.name)
        );
    }
    let groups: BTreeSet<_> = folded
        .iter()
        .flat_map(|(_, s)| s.regions.iter().filter_map(|r| r.shared))
        .collect();
    for id in groups {
        let k = id + 1;
        let _ = write!(out, " · <a href=\"#shared-{k}\">#{k}</a>");
    }
    out.push_str("</p></div>");
}

/// Variants of the component set that look different from the shown one, folded.
fn other_variants(out: &mut String, group: &crate::SharedChange) {
    let Some(c) = &group.component else { return };
    let shown: Vec<_> = c
        .variants
        .iter()
        .skip(1)
        .filter(|v| v.old_png.is_some())
        .collect();
    if shown.is_empty() {
        return;
    }
    out.push_str("<details><summary>Other variants</summary>");
    for v in shown {
        let _ = write!(out, "<p class=\"none\">{}</p>", escape(&v.name));
        compare_sized(
            out,
            (&v.old_png, &v.new_png),
            v.image_width.zip(v.image_height),
            &v.name,
        );
    }
    out.push_str("</details>");
}
