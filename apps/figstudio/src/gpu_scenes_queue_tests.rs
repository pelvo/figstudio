//! The main thread's scene queue with the real worker queue (`tile_queue::Queue`), tile cache
//! and tab bookkeeping, frame by frame as the browser runs them: views that move on leave no
//! scenes behind, the queue stays bounded, the view centre renders first, a dropped scene's
//! tile is rendered again when asked for again, and every scene's byte credits come back.

use std::path::PathBuf;

use crate::recordings::Pixels;
use crate::tab::Tab;
use crate::tile_cache::Cache;
use crate::tile_queue::{Msg, Queue};
use crate::tiles::{self, Key, Range};
use crate::tiles_view::received;

use super::*;

/// Tiles the worker renders between two UI frames in this model.
const WORKER_PER_FRAME: usize = 20;

/// The UI, the worker queue and the main thread's scenes of one tab.
struct Model {
    tab: Tab,
    cache: Cache<()>,
    worker: Queue,
    pending: Pending<Item>,
    /// Every tile the worker rendered, in order.
    rendered: Vec<Key>,
    /// Every scene the main thread rendered, in order.
    shown: Vec<Key>,
    /// The byte credits the worker's scenes reserve.
    credits: Credits,
    /// Tiles drawn on the CPU because no credits were left.
    cpu: Vec<Key>,
    /// Bytes each of the worker's scenes reserves.
    scene_bytes: usize,
}

impl Model {
    fn new() -> Self {
        let doc = std::sync::Arc::new(
            figstudio_fig::Document::open(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/structure.fig"
            ))
            .unwrap(),
        );
        Model {
            tab: Tab::new(1, PathBuf::from("t.fig"), doc, None, Duration::ZERO),
            cache: Cache::new(1000),
            worker: Queue::default(),
            pending: Pending::default(),
            rendered: Vec::new(),
            shown: Vec::new(),
            credits: Credits::default(),
            cpu: Vec::new(),
            // Small enough that the credits never run out (the queue's own logic).
            scene_bytes: 1 << 20,
        }
    }

    /// The 10 x 6 tiles of view `v` (views do not overlap).
    fn view(&self, v: i32) -> Range {
        Range {
            level: 0,
            x0: 100 * v,
            y0: 0,
            x1: 100 * v + 10,
            y1: 6,
        }
    }

    /// One frame on view `range`, in the app's order: the main thread renders scenes
    /// (`Tiles::pump_gpu`, pruned to the last plan), plans and asks for missing tiles
    /// (`Tiles::show`), then the worker renders some and their scenes arrive (`accept`).
    fn frame(&mut self, range: Range) {
        let (doc, page) = (self.tab.doc.instance, self.tab.page);
        let (tab, cache, shown) = (&mut self.tab, &mut self.cache, &mut self.shown);
        self.pending
            .prune(cache.plan(), |(done, _)| received(tab, &done));
        assert!(self.pending.len() <= MAX_PENDING, "{}", self.pending.len());
        self.pending.pump(|(done, _)| {
            received(tab, &done);
            shown.push(done.key);
            cache.insert(done.key, (), done.stats, None);
            Duration::from_millis(1)
        });
        self.cache.begin_frame(doc, page, range);
        let wanted = tiles::wanted(&self.cache, range, doc, page, 0);
        if let Some(batch) = tiles::next_batch(wanted, &self.tab.tile_requests) {
            let job = crate::worker::RenderJob {
                tab: self.tab.id,
                doc: self.tab.doc.clone(),
                source: None,
                root: page,
                background: None,
                images: 0,
                tiles: batch.iter().copied().collect(),
                received: std::mem::take(&mut self.tab.tiles_received),
            };
            self.worker.take(Msg::Batch(job));
            self.tab.tile_requests = batch;
        }
        for _ in 0..WORKER_PER_FRAME {
            if self.worker.is_empty() {
                break;
            }
            let Some((job, key)) = self.worker.next() else {
                continue;
            };
            let (tab, images) = (job.tab, job.images);
            self.worker.sent(tab, key, images);
            self.rendered.push(key);
            let ui = &mut self.tab;
            let plan = self.cache.plan();
            match scene(&self.credits, self.scene_bytes, tab, key, images) {
                Some(item) => self.pending.push(item, plan, |(d, _)| received(ui, &d)),
                // No credits left: drawn on the CPU (counted) and stored at once (`accept`).
                None => {
                    ui.tile_requests.retain(|k| *k != key);
                    ui.tiles_received.push((key, images));
                    self.cpu.push(key);
                    self.cache.insert(key, (), Default::default(), None);
                }
            }
        }
    }
}

/// A queued GPU tile and the credits its scene holds (in the browser, `GpuTexture::bytes`;
/// natively a tile's pixels are a wgpu texture, so the model carries them alongside).
type Item = (RenderDone, Reservation);

impl Keyed for Item {
    fn key(&self) -> Key {
        self.0.key
    }
}

/// A tile holding `size` bytes of `credits`, or `None` when they are spent.
fn scene(credits: &Credits, size: usize, tab: u64, key: Key, images: u64) -> Option<Item> {
    let bytes = credits.reserve(size)?;
    let done = RenderDone {
        tab,
        key,
        images,
        pixels: Pixels::Cpu {
            size: [0, 0],
            rgba: Vec::new(),
        },
        stats: Default::default(),
        elapsed: Duration::ZERO,
    };
    Some((done, bytes))
}

#[test]
fn views_that_move_on_leave_no_scenes_and_the_centre_renders_first() {
    let mut m = Model::new();
    // Ten views, one frame each: the worker renders 20 tiles of each.
    for v in 0..10 {
        let range = m.view(v);
        m.frame(range);
    }
    // Then the user stays on view 10.
    let last = m.view(10);
    m.frame(last);
    m.frame(last);
    // Scenes are pruned to the previous frame's plan (`pump_gpu` runs before `show`), so each
    // view change lets through at most one frame's worth of the old view's scenes.
    let stale = m.shown.iter().filter(|k| !last.contains(k)).count();
    assert!(stale <= TILES_PER_FRAME * 10, "{stale} stale scenes shown");
    let queued: Vec<Key> = m.pending.queue.iter().map(Keyed::key).collect();
    assert!(
        queued.iter().all(|k| last.contains(k)),
        "scenes of other views still queued: {queued:?}"
    );
    // Staying on the view, every tile of it arrives, each rendered once.
    for _ in 0..40 {
        m.frame(last);
    }
    let wanted = tiles::wanted(&m.cache, last, m.tab.doc.instance, m.tab.page, 0);
    assert!(wanted.is_empty(), "{} tiles never arrived", wanted.len());
    let mut of_last: Vec<_> = m.rendered.iter().filter(|k| last.contains(k)).collect();
    let all = of_last.len();
    of_last.sort_by_key(|k| (k.y, k.x));
    of_last.dedup();
    assert_eq!(all, of_last.len(), "a tile of the view was rendered twice");
}

#[test]
fn a_dropped_scene_is_rendered_again_when_its_view_returns() {
    let mut m = Model::new();
    let (a, b) = (m.view(0), m.view(1));
    m.frame(a);
    m.frame(b); // view 0's 20 scenes arrived in the first frame and are dropped now
    // 4 of them were shown in that frame (still planned then), the rest dropped.
    let dropped: Vec<Key> = m.rendered[..WORKER_PER_FRAME]
        .iter()
        .copied()
        .filter(|k| m.cache.get(k).is_none())
        .collect();
    assert_eq!(dropped.len(), WORKER_PER_FRAME - TILES_PER_FRAME);
    for _ in 0..40 {
        m.frame(a);
    }
    let wanted = tiles::wanted(&m.cache, a, m.tab.doc.instance, m.tab.page, 0);
    assert!(
        wanted.is_empty(),
        "{} tiles of the view never came back",
        wanted.len()
    );
    for k in &dropped {
        let times = m.rendered.iter().filter(|r| *r == k).count();
        assert_eq!(times, 2, "{k:?}: dropped, then rendered again");
    }
}

/// A 20 x 20 plan, planned in `m`'s cache, and its tiles nearest the centre first.
fn big_plan(m: &mut Model) -> Vec<Key> {
    let range = Range {
        level: 0,
        x0: 0,
        y0: 0,
        x1: 20,
        y1: 20,
    };
    m.cache.begin_frame(m.tab.doc.instance, m.tab.page, range);
    range.by_distance(m.tab.doc.instance, m.tab.page)
}

#[test]
fn a_frame_renders_at_most_a_few_scenes_and_the_queue_is_bounded() {
    let mut m = Model::new();
    let by_distance = big_plan(&mut m);
    // Farthest first, as a worker far behind would send them; free credits for each.
    let (plan, mut dropped) = (m.cache.plan(), 0);
    for &key in by_distance.iter().rev() {
        let item = scene(&Credits::default(), 0, 1, key, 0).unwrap();
        m.pending.push(item, plan, |_| dropped += 1);
        assert!(m.pending.len() <= MAX_PENDING, "{} scenes", m.pending.len());
    }
    assert_eq!((m.pending.len(), dropped), (MAX_PENDING, 400 - MAX_PENDING));
    let mut first = Vec::new();
    m.pending.pump(|(d, _)| {
        first.push(d.key);
        Duration::from_millis(1)
    });
    // The four centre tiles (equally near, in any order).
    assert!(
        first
            .iter()
            .all(|k| by_distance[..TILES_PER_FRAME].contains(k)),
        "{first:?}"
    );
    // A slow scene ends the frame's GPU work, but one always goes.
    let mut n = 0;
    m.pending.pump(|_| {
        n += 1;
        TIME_PER_FRAME
    });
    assert_eq!(n, 1);
}

/// Rendered, pruned (the view moved on) or forgotten (the document closed): each gives its
/// credits back, and the credits never exceed the budget.
#[test]
fn every_way_out_of_the_queue_gives_the_bytes_back() {
    let mut m = Model::new();
    // The largest scenes the worker builds: the budget holds 8 of them.
    m.scene_bytes = MAX_SCENE_BYTES;
    let view = m.view(0);
    for _ in 0..40 {
        m.frame(view);
        assert!(m.credits.used() <= crate::gpu_credits::BUDGET);
    }
    // Every tile came (some on the CPU, when the credits were spent); nothing is held after.
    assert!(
        !m.cpu.is_empty(),
        "20 scenes a frame of 8 MiB outrun 64 MiB"
    );
    let wanted = tiles::wanted(&m.cache, view, m.tab.doc.instance, m.tab.page, 0);
    assert!(wanted.is_empty(), "{} tiles never came", wanted.len());
    assert_eq!(m.credits.used(), 0, "rendered scenes keep no credits");
    // Scenes of a view the user left are pruned: their credits come back.
    m.frame(m.view(1));
    assert!(m.credits.used() > 0);
    m.frame(m.view(2));
    m.frame(m.view(2));
    let left = m.pending.len() * MAX_SCENE_BYTES;
    assert_eq!(
        m.credits.used(),
        left,
        "only the queued scenes hold credits"
    );
    // The document closes: its queued scenes go, with their credits.
    m.pending.forget(m.tab.doc.instance);
    assert_eq!(m.credits.used(), 0);
}
