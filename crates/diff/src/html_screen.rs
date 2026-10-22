//! Existing area cards, now scoped to a standalone screen and its locator.
use crate::html::{escape, plural};
use crate::html_changes::PageLines;
use crate::html_lines::{ChangeLine, count_lines, render_lines};
use crate::html_pictures::{compare, locator, picture};
use crate::{Region, ScreenDiff, ScreenStatus, SharedChange};
use std::{collections::BTreeSet, fmt::Write};

fn area_heading(out: &mut String, region: &Region, k: usize, layers: usize) {
    let _ = write!(
        out,
        "<div class=\"ah\"><b>Area {k}</b><span>{} px</span><span>x {} · y {} · {}×{}</span>",
        region.pixels_changed, region.x, region.y, region.width, region.height,
    );
    // Area cards without lines sit under their own "no matched change line" heading.
    if layers > 0 {
        let _ = write!(
            out,
            "<span>{}</span>",
            plural(layers, "change line", "change lines")
        );
    }
    out.push_str("</div>");
}

pub(crate) fn screen_lines<'a>(
    screen: &'a ScreenDiff,
    lines: &'a PageLines,
) -> impl Iterator<Item = &'a ChangeLine> {
    lines.items.iter().filter(move |l| {
        screen.nodes.contains(&l.id)
            && crate::screen_model::counts(l, screen.matched_by, (screen.old_id, screen.new_id))
    })
}

pub(crate) fn only_shared(screen: &ScreenDiff, lines: &PageLines) -> bool {
    screen.status == ScreenStatus::Changed
        && !screen.regions.is_empty()
        && screen.regions.iter().all(|r| r.shared.is_some())
        && screen_lines(screen, lines).next().is_none()
}

fn no_visible_change(out: &mut String, screen: &ScreenDiff) {
    if screen.pixels_changed == Some(0) {
        out.push_str("<p class=\"none\">No visible change: these edits draw no different pixels (for example clipped or hidden layers).</p>");
    }
    if screen.thumb_old.is_some() || screen.thumb_new.is_some() {
        out.push_str(
            "<div class=\"cmp thumbs\" tabindex=\"0\" aria-label=\"Screen: Before / After\">",
        );
        picture(out, &screen.thumb_old, "Before", "bf");
        picture(out, &screen.thumb_new, "After", "af");
        out.push_str(
            "<span class=\"hint\">Showing After · hover, focus or press to see Before</span></div>",
        );
    }
}

pub(crate) fn screen_section(
    out: &mut String,
    n: usize,
    k: usize,
    screen: &ScreenDiff,
    lines: &PageLines,
    shared: &[SharedChange],
) {
    let status = format!("{:?}", screen.status);
    let _ = write!(
        out,
        "<div class=\"sec screen\" id=\"page-{n}-s{k}\"><div class=\"sh\"><h3>{}</h3><span class=\"badge {}\">{status}</span>",
        escape(&screen.name),
        status.to_lowercase()
    );
    if screen.moved {
        out.push_str("<span class=\"tag\">moved</span>");
    }
    if screen.status != ScreenStatus::Changed {
        out.push_str("</div><div class=\"screen-thumb\">");
        if screen.status == ScreenStatus::Added {
            picture(out, &screen.thumb_new, "After", "af");
        } else {
            picture(out, &screen.thumb_old, "Before", "bf");
        }
        out.push_str("</div>");
        if let Some(id) = screen.new_id.or(screen.old_id) {
            let added = screen.status == ScreenStatus::Added;
            render_lines(
                out,
                std::iter::once(&ChangeLine {
                    id,
                    sign: if added { "+" } else { "-" },
                    class: if added { "add" } else { "del" },
                    name: screen.name.clone(),
                    kind: screen.kind.clone(),
                    path: screen.name.clone(),
                    rows: Vec::new(),
                }),
            );
        }
        out.push_str("</div>");
        return;
    }
    out.push_str("</div>");
    if screen.regions.is_empty() {
        // Layer edits that draw nothing at this scale (clipped, hidden, sub-pixel) still get
        // a picture of the screen, so the reader can see why no area is boxed.
        no_visible_change(out, screen);
    }
    let groups: BTreeSet<_> = screen.regions.iter().filter_map(|r| r.shared).collect();
    for id in groups {
        if let Some(group) = shared.get(id) {
            let k = id + 1;
            let _ = write!(
                out,
                "<p class=\"none\"><a href=\"#shared-{k}\">Shared change #{k}</a> · also in {}</p>",
                plural(
                    group.count.saturating_sub(1),
                    "other screen",
                    "other screens"
                )
            );
        }
    }
    let shared_nodes: BTreeSet<_> = screen
        .regions
        .iter()
        .filter(|r| r.shared.is_some())
        .flat_map(|r| r.nodes.iter().copied())
        .collect();
    let area_lines = || screen_lines(screen, lines).filter(|l| !shared_nodes.contains(&l.id));
    let (full, mini): (Vec<_>, Vec<_>) = screen
        .regions
        .iter()
        .enumerate()
        .filter(|(_, r)| r.shared.is_none())
        .partition(|(_, r)| area_lines().any(|l| r.nodes.contains(&l.id)));
    let mut used = BTreeSet::new();
    for (j, region) in full {
        let j = j + 1;
        let selected = || area_lines().filter(|l| region.nodes.contains(&l.id));
        let _ = write!(out, "<article class=\"area\" id=\"page-{n}-s{k}-a{j}\">");
        area_heading(out, region, j, count_lines(selected()));
        compare(out, region, j);
        out.push_str("<div class=\"info\">");
        locator(out, screen, region);
        render_lines(out, selected());
        used.extend(selected().map(|l| l.id));
        out.push_str("</div></article>");
    }
    if !mini.is_empty() {
        // "more" only when full area cards come before these.
        let (one, many) = if used.is_empty() {
            ("area", "areas")
        } else {
            ("more area", "more areas")
        };
        let _ = write!(
            out,
            "<p class=\"subh\">{} with no matched change line</p><div class=\"minis\">",
            plural(mini.len(), one, many)
        );
        for (j, region) in mini {
            let j = j + 1;
            let _ = write!(out, "<div class=\"mini\" id=\"page-{n}-s{k}-a{j}\">");
            area_heading(out, region, j, 0);
            compare(out, region, j);
            out.push_str("</div>");
        }
        out.push_str("</div>");
    }
    let rest = || screen_lines(screen, lines).filter(|l| !used.contains(&l.id));
    if rest().next().is_some() {
        out.push_str(if screen.regions.is_empty() {
            "<h4 class=\"sub\">Changes in this screen</h4>"
        } else {
            "<h4 class=\"sub\">Other changes in this screen</h4>"
        });
        render_lines(out, rest());
    }
    out.push_str("</div>");
}
