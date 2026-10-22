//! The only property rows the HTML report shows: text, font and paint colour edits.
//! Everything else stays in report.json (see `Design::other_props`).

use figstudio_fig::Val;

use crate::html::escape;
use crate::html_component::{content_row, inherited_row, is_derived, is_version};
use crate::html_kind::{Kind, classify, label_of};
use crate::html_lines::ChangeLine;
use crate::html_swatch::{Channels, swatch};
use crate::{Category, NodeDiff, PageDiff, PropertyChange};

/// A move or reposition: only `transform.m02`/`m12` (or the sibling order) changed.
pub(crate) fn is_position(field: &str) -> bool {
    field.starts_with("parentIndex")
        || field.ends_with("transform.m02")
        || field.ends_with("transform.m12")
}

/// `(absent) → 1` / `(absent) → true`: a default that was only written out.
fn is_new_default(change: &PropertyChange) -> bool {
    change.old.is_none()
        && (matches!(change.new, Some(Val::Bool(true))) || num(change.new.as_ref()) == Some(1.0))
}

fn arrow(old: &str, new: &str) -> String {
    format!(
        "<span class=\"o\">{old}</span><span class=\"ar\">→</span><span class=\"n\">{new}</span>"
    )
}

pub(crate) fn plain(v: Option<&Val>) -> String {
    match v {
        None => "(absent)".into(),
        Some(Val::Str(s)) => escape(s),
        Some(Val::Int(i)) => i.to_string(),
        Some(Val::Float(f)) => f.to_string(),
        Some(Val::Bool(b)) => b.to_string(),
        Some(Val::Enum(e)) => escape(e),
        Some(other) => escape(&serde_json::to_string(other).unwrap_or_default()),
    }
}

pub(crate) fn num(v: Option<&Val>) -> Option<f64> {
    match v? {
        Val::Float(f) => Some(*f),
        Val::Int(i) => Some(*i as f64),
        _ => None,
    }
}

struct ColourRow<'a> {
    prefix: String,
    label: &'static str,
    old: Channels<'a>,
    new: Channels<'a>,
}

fn node_rows(node: &NodeDiff, other_props: &mut usize) -> Vec<String> {
    let (mut text, mut font, mut paint) = (Vec::new(), Vec::new(), Vec::new());
    let mut colours: Vec<ColourRow> = Vec::new();
    let (mut derived, mut component) = (Vec::new(), None);
    let text_shown = node.text.as_ref().filter(|t| t.old != t.new);
    if let Some(t) = text_shown {
        text.push(format!(
            "<span class=\"kv\"><span class=\"k\">text</span>{}</span>",
            arrow(
                &format!("「{}」", escape(&t.old)),
                &format!("「{}」", escape(&t.new)),
            )
        ));
    }
    let in_runs = |c: &PropertyChange| {
        node.text.as_ref().is_some_and(|t| {
            t.runs
                .iter()
                .any(|r| r.changes.iter().any(|x| x.field == c.field))
        })
    };
    let runs = node
        .text
        .iter()
        .flat_map(|t| t.runs.iter().flat_map(|r| r.changes.iter()));
    for change in node.changes.iter().filter(|c| !in_runs(c)).chain(runs) {
        if text_shown.is_some() && change.category == Category::Text {
            continue;
        }
        if is_derived(change) {
            derived.push(change);
            continue;
        }
        if is_version(change) {
            if node.kind == "SYMBOL" {
                component = Some("component updated".to_string());
            } else {
                *other_props += 1;
            }
            continue;
        }
        if is_position(&change.field) {
            continue;
        }
        let kind = classify(change);
        if matches!(kind, Kind::Paint) && is_new_default(change) {
            *other_props += 1;
            continue;
        }
        let row = |label: &str| {
            format!(
                "<span class=\"kv\"><span class=\"k\">{}:</span>{}</span>",
                escape(label),
                arrow(&plain(change.old.as_ref()), &plain(change.new.as_ref()))
            )
        };
        match kind {
            Kind::Font => font.push(row(&label_of(&change.field))),
            Kind::Paint => {
                let segs: Vec<&str> = change.field.split('.').collect();
                let n = segs.len();
                let place = if segs[n - 3] == "strokePaints" {
                    "Stroke"
                } else {
                    "Fill"
                };
                paint.push(row(&format!("{place} {}", segs[n - 1])));
            }
            Kind::Colour { prefix, label, ch } => {
                let at = colours
                    .iter()
                    .position(|c| c.prefix == prefix)
                    .unwrap_or_else(|| {
                        colours.push(ColourRow {
                            prefix,
                            label,
                            old: [None; 4],
                            new: [None; 4],
                        });
                        colours.len() - 1
                    });
                colours[at].old[ch] = change.old.as_ref();
                colours[at].new[ch] = change.new.as_ref();
            }
            Kind::Hidden => *other_props += 1,
        }
    }
    let colour_rows = colours.iter().map(|c| {
        format!(
            "<span class=\"kv\"><span class=\"k\">{}:</span>{}</span>",
            c.label,
            arrow(&swatch(&c.old, &c.new), &swatch(&c.new, &c.old))
        )
    });
    let content = content_row(&derived);
    *other_props += usize::from(!derived.is_empty() && content.is_none());
    let rows = text.into_iter().chain(font).chain(colour_rows).chain(paint);
    rows.chain(
        component
            .into_iter()
            .map(|r| format!("<span class=\"tag\">{r}</span>")),
    )
    .chain(content.into_iter().map(|r| {
        format!("<span class=\"tag in\">inside component</span><span class=\"tag\">{r}</span>")
    }))
    .collect()
}

/// Human-visible lines retain their layer identity until grouped inside each area.
#[derive(Default)]
pub(crate) struct PageLines {
    pub items: Vec<ChangeLine>,
    pub other_props: usize,
    pub other_layers: usize,
}

pub(crate) fn page_lines(page: &PageDiff) -> PageLines {
    let mut out = PageLines::default();
    for (sign, class, nodes) in [("+", "add", &page.added), ("-", "del", &page.removed)] {
        out.items.extend(nodes.iter().map(|n| ChangeLine {
            id: n.id,
            sign,
            class,
            name: n.name.clone(),
            kind: n.kind.clone(),
            path: n.path.join(" / "),
            rows: Vec::new(),
        }));
    }
    let has_regions = page
        .screens
        .iter()
        .any(|s| s.pixels_changed > Some(0) && !s.regions.is_empty());
    for node in &page.changed {
        let before = out.other_props;
        let mut rows = node_rows(node, &mut out.other_props);
        out.other_layers += usize::from(out.other_props > before);
        let in_area = page
            .screens
            .iter()
            .any(|s| s.regions.iter().any(|r| r.nodes.contains(&node.id)));
        if rows.is_empty() && has_regions && in_area && inherited_row(node) {
            out.other_layers -= usize::from(out.other_props > before);
            out.other_props = before;
            rows.push(
                "<span class=\"tag\">shared component changed (see Changed areas)</span>".into(),
            );
        }
        if !rows.is_empty() {
            out.items.push(ChangeLine {
                id: node.id,
                sign: "~",
                class: "mod",
                name: node.name.clone(),
                kind: node.kind.clone(),
                path: node.path.join(" / "),
                rows,
            });
        }
    }
    out
}
