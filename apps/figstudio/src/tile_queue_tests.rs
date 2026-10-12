use std::sync::{Arc, OnceLock};

use super::*;
use figstudio_fig::{Document, Guid};

fn doc() -> Arc<Document> {
    static DOC: OnceLock<Arc<Document>> = OnceLock::new();
    DOC.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/structure.fig"
        );
        Arc::new(Document::open(path).unwrap())
    })
    .clone()
}

fn key(x: i32) -> Key {
    Key {
        doc: 1,
        page: Guid(0, 1),
        level: 0,
        x,
        y: 0,
    }
}

fn batch(tab: u64, xs: &[i32], received: &[i32]) -> Msg {
    Msg::Batch(RenderJob {
        tab,
        doc: doc(),
        source: None,
        root: Guid(0, 1),
        background: None,
        images: 0,
        tiles: xs.iter().map(|&x| key(x)).collect(),
        received: received.iter().map(|&x| (key(x), 0)).collect(),
    })
}

/// Renders (pops and "sends") every queued tile; returns `(tab, x)` in order.
fn drain(q: &mut Queue) -> Vec<(u64, i32)> {
    let mut out = Vec::new();
    while !q.is_empty() {
        let Some((job, k)) = q.next() else { continue };
        let (tab, images) = (job.tab, job.images);
        out.push((tab, k.x));
        q.sent(tab, k, images);
    }
    out
}

#[test]
fn a_tile_planned_before_its_result_arrived_is_not_rendered_twice() {
    let mut q = Queue::default();
    q.take(batch(1, &[0, 1], &[]));
    let (job, k) = q.next().unwrap();
    let tab = job.tab;
    q.sent(tab, k, 0);
    // The UI planned this batch before it saw tile 0.
    q.take(batch(1, &[0, 1, 2], &[]));
    assert_eq!(drain(&mut q), vec![(1, 1), (1, 2)]);
}

#[test]
fn a_tile_received_then_evicted_is_rendered_again() {
    let mut q = Queue::default();
    q.take(batch(1, &[0], &[]));
    assert_eq!(drain(&mut q), vec![(1, 0)]);
    // The UI got tile 0, evicted it (another tab filled the cache) and asks again.
    q.take(batch(1, &[0], &[0]));
    assert_eq!(drain(&mut q), vec![(1, 0)]);
}

#[test]
fn another_tabs_request_for_the_same_tile_is_rendered() {
    let mut q = Queue::default();
    q.take(batch(1, &[0], &[]));
    assert_eq!(drain(&mut q), vec![(1, 0)]);
    // Same document in a second tab, before tab 1 acknowledged anything.
    q.take(batch(2, &[0], &[]));
    assert_eq!(drain(&mut q), vec![(2, 0)]);
}

#[test]
fn a_newer_batch_replaces_the_rest_of_the_old_one_and_goes_first() {
    let mut q = Queue::default();
    assert_eq!(q.take(batch(1, &[0, 1, 2], &[])), 0);
    assert_eq!(q.take(batch(2, &[5], &[])), 0);
    assert_eq!(q.take(batch(1, &[3], &[])), 1);
    assert_eq!(drain(&mut q), vec![(1, 3), (2, 5)]);
}

#[test]
fn closing_a_tab_drops_its_queued_tiles_and_history() {
    let mut q = Queue::default();
    q.take(batch(1, &[0, 1, 2], &[]));
    q.take(batch(2, &[7, 8], &[]));
    let (job, k) = q.next().unwrap();
    let tab = job.tab;
    q.sent(tab, k, 0);
    assert_eq!(q.take(Msg::Close { tab: 2 }), 1);
    assert_eq!(drain(&mut q), vec![(1, 0), (1, 1), (1, 2)]);
    assert!(q.unacked.iter().all(|u| u.0 == 1));
    assert_eq!(q.take(Msg::Close { tab: 1 }), 0);
    assert!(q.unacked.is_empty(), "history kept after close");
}

#[test]
fn a_tile_stays_skipped_until_the_ui_acknowledges_it() {
    let mut q = Queue::default();
    q.take(batch(1, &[0, 1], &[]));
    let (job, k) = q.next().unwrap();
    let tab = job.tab;
    q.sent(tab, k, 0);
    // Two batches planned before the UI saw tile 0.
    q.take(batch(1, &[0, 1], &[]));
    q.take(batch(1, &[0, 1, 2], &[]));
    assert_eq!(drain(&mut q), vec![(1, 1), (1, 2)]);
}
