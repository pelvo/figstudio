//! The region text, on `structure.fig` and on a document built here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::link::parse_link;

use super::*;

const STRUCTURE: &[u8] = include_bytes!("../../../tests/fixtures/structure.fig");

fn structure() -> Tab {
    let doc = Document::from_bytes(STRUCTURE).unwrap();
    Tab::new(
        1,
        PathBuf::from("/private/dir/structure.fig"),
        Arc::new(doc),
        None,
        Duration::ZERO,
    )
}

fn whole_page(tab: &Tab) -> [f64; 4] {
    let b = figstudio_render::content_bounds(&tab.doc, tab.page).unwrap();
    [b.x0, b.y0, b.width(), b.height()]
}

#[test]
fn the_text_is_link_then_file_and_page_then_node_rows() {
    let tab = structure();
    let text = region_text(&tab, tab.page, whole_page(&tab)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("figstudio://structure?page="),
        "{text}"
    );
    assert_eq!(lines[1], "file: structure.fig · page: Page 1");
    assert!(lines[2].starts_with("nodes ("), "{text}");
    assert!(lines[2].ends_with(" top-level first):"), "{text}");
    let row = lines[3];
    assert!(row.contains(" \"") && row.ends_with(')'), "{row}");
    assert!(row.split(' ').next().unwrap().contains(':'), "{row}");
}

#[test]
fn the_link_reads_back_to_the_same_page_and_rect() {
    let tab = structure();
    let rect = [10.5, -20.0, 300.25, 120.0];
    let text = region_text(&tab, tab.page, rect).unwrap();
    let link = parse_link(text.lines().next().unwrap()).unwrap();
    assert_eq!(link.page, Some(tab.page));
    assert_eq!(link.rect, Some(rect));
}

#[test]
fn the_count_line_matches_the_rows() {
    let tab = structure();
    let text = region_text(&tab, tab.page, whole_page(&tab)).unwrap();
    let rows = text.lines().skip(3).count();
    let head = text.lines().nth(2).unwrap();
    assert!(head.starts_with(&format!("nodes ({rows} of ")), "{head}");
    assert!(rows > 0 && rows <= 50);
}

#[test]
fn a_region_with_nothing_in_it_still_has_a_header() {
    let tab = structure();
    let text = region_text(&tab, tab.page, [1.0e6, 1.0e6, 5.0, 5.0]).unwrap();
    assert!(text.ends_with("nodes (0 of 0, top-level first):"), "{text}");
}

#[test]
fn a_rect_a_link_cannot_hold_is_an_error_message() {
    let tab = structure();
    assert!(region_text(&tab, tab.page, [0.0, 0.0, 0.0, 5.0]).is_err());
    assert!(region_text(&tab, tab.page, [f64::NAN, 0.0, 5.0, 5.0]).is_err());
}

#[test]
fn top_level_rows_come_before_their_children() {
    let tab = structure();
    let text = region_text(&tab, tab.page, whole_page(&tab)).unwrap();
    let ids: Vec<Guid> = text
        .lines()
        .skip(3)
        .map(|l| {
            let id = l.split(' ').next().unwrap();
            let (a, b) = id.split_once(':').unwrap();
            Guid(a.parse().unwrap(), b.parse().unwrap())
        })
        .collect();
    let depth = |g: Guid| {
        let mut d = 0;
        let mut cur = tab.doc.node(g).and_then(|n| n.parent);
        while let Some(p) = cur {
            d += 1;
            cur = tab.doc.node(p).and_then(|n| n.parent);
        }
        d
    };
    assert!(
        ids.windows(2).all(|w| depth(w[0]) <= depth(w[1])),
        "{ids:?}"
    );
}
