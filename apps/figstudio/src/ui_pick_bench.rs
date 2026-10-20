//! Manual release benchmark: FIGSTUDIO_PICK_BENCH_FILE points at an untracked synthetic file.
use crate::{canvas_pick, instance_pick, tab::Tab};
use figstudio_fig::Document;
use figstudio_render::{kurbo::Point, node_bounds};
use std::{
    hint::black_box,
    sync::Arc,
    time::{Duration, Instant},
};

fn measure(label: &str, batch: u32, mut f: impl FnMut()) {
    let mut samples = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        for _ in 0..batch {
            f();
        }
        samples.push(start.elapsed().as_secs_f64() * 1000.0 / f64::from(batch));
    }
    samples.sort_by(f64::total_cmp);
    println!(
        "{label}: median_ms={:.6} min_ms={:.6} max_ms={:.6}",
        samples[5], samples[0], samples[9]
    );
}

#[test]
#[ignore = "manual benchmark; requires FIGSTUDIO_PICK_BENCH_FILE"]
fn instance_pick_release_benchmark() {
    let file = std::env::var("FIGSTUDIO_PICK_BENCH_FILE").unwrap();
    let doc = Arc::new(Document::open(std::path::Path::new(&file)).unwrap());
    let page = doc.pages[0];
    let instance = doc
        .nodes
        .iter()
        .find(|n| n.kind == "INSTANCE" && doc.page_of(n.id) == Some(page))
        .unwrap()
        .id;
    let bounds = node_bounds(&doc, instance).unwrap();
    let point = Point::new((bounds.x0 + bounds.x1) / 2.0, (bounds.y0 + bounds.y1) / 2.0);
    let start = Instant::now();
    let mut tab = Tab::new(1, "synthetic.fig".into(), doc, None, Duration::ZERO);
    println!(
        "nodes={} samples=10 tab_preparation_ms={:.6}",
        tab.doc.nodes.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    tab.hover = Some(instance);
    tab.selection = tab.hover;
    measure("page hit", 1, || {
        black_box(figstudio_render::hit_chain(&tab.doc, page, point));
    });
    measure("inner click", 1, || {
        black_box(tab.pick_cache.with(&tab.doc, |renderer, _| {
            instance_pick::hit_chain_with(&tab.doc, renderer, instance, point)
        }));
    });
    let inner = tab.pick_cache.with(&tab.doc, |renderer, _| {
        instance_pick::hit_chain_with(&tab.doc, renderer, instance, point)
    })[0]
        .clone();
    tab.inner = Some(inner.clone());
    black_box(canvas_pick::hover(&tab, Some(point), false));
    measure("repeated hover (batch=10000)", 10000, || {
        black_box(canvas_pick::hover(&tab, Some(point), false));
    });
    let start = Instant::now();
    assert!(black_box(tab.pick_cache.corners(&tab.doc, &inner)).is_some());
    println!(
        "first outline_ms={:.6}",
        start.elapsed().as_secs_f64() * 1000.0
    );
    measure("outline (batch=10000)", 10000, || {
        black_box(tab.pick_cache.corners(&tab.doc, &inner));
    });
    println!("work={:?}", tab.pick_cache.work(&tab.doc));
}
