//! Coverage and eviction across frames: batches settle, planned tiles stay, fallback gives way.

use super::*;
use figstudio_render::Stats;
use figstudio_render::kurbo::Rect;

use crate::tiles::{Range, plan, visible_page, wanted};

const PAGE: Guid = Guid(0, 1);

fn key(level: i32, x: i32, y: i32) -> Key {
    Key {
        doc: 7,
        page: PAGE,
        level,
        x,
        y,
    }
}

/// Renders every wanted tile in batches of `per_batch`, as the worker would, until nothing is
/// wanted; returns the planned range and how many batches it took (or `None` if it stalls).
fn settle(
    cache: &mut Cache<()>,
    level: i32,
    visible: Rect,
    per_batch: usize,
) -> (Range, Option<usize>) {
    let range = plan(level, visible, cache.budget());
    for round in 0..100 {
        cache.begin_frame(7, PAGE, range);
        for k in cache.draw_order(7, PAGE, range.level, visible) {
            cache.touch(&k);
        }
        let w = wanted(cache, range, 7, PAGE, 0);
        if w.is_empty() {
            return (range, Some(round));
        }
        for k in w.into_iter().take(per_batch) {
            cache.insert(k, (), Stats::default(), None);
        }
    }
    (range, None)
}

#[test]
fn successive_batches_cover_a_view_bigger_than_half_the_cache() {
    // The browser budget (120 tiles) and an 8K canvas at 1 ppp: 15 x 9 tiles at level 0.
    let mut c = Cache::new(120);
    let visible = visible_page(1.0, [0.0, 0.0], [7680.0, 4320.0]);
    let (range, rounds) = settle(&mut c, 0, visible, 16);
    assert!(rounds.is_some(), "never covered the view");
    assert!(
        range.level < 0,
        "a view over half the cache must use a coarser level"
    );
    let shown = Range::covering(range.level, visible);
    for k in shown.by_distance(7, PAGE) {
        assert!(c.get(&k).is_some(), "{k:?} never rendered");
    }
}

#[test]
fn a_view_within_half_the_cache_keeps_its_level_and_ring() {
    let mut c = Cache::new(240);
    let visible = visible_page(1.0, [0.0, 0.0], [2560.0, 1440.0]);
    let (range, rounds) = settle(&mut c, 0, visible, 8);
    assert!(rounds.is_some());
    assert_eq!(range, Range::covering(0, visible).grow(1));
}

#[test]
fn tiles_drawn_last_frame_are_not_evicted_by_new_ones() {
    let mut c = Cache::new(4);
    for x in 0..4 {
        c.insert(key(0, x, 0), (), Stats::default(), None);
    }
    c.begin_frame(
        7,
        PAGE,
        Range {
            level: 0,
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
        },
    );
    c.touch(&key(0, 0, 0));
    c.touch(&key(0, 1, 0));
    // Results arrive before the next frame is drawn.
    for x in 10..14 {
        c.insert(key(0, x, 0), (), Stats::default(), None);
    }
    assert_eq!(c.len(), 4);
    assert!(c.get(&key(0, 0, 0)).is_some() && c.get(&key(0, 1, 0)).is_some());
}

#[test]
fn outer_tiles_are_wanted_once_the_centre_is_cached() {
    let mut c: Cache<()> = Cache::new(8);
    let r = Range {
        level: 0,
        x0: 0,
        y0: 0,
        x1: 3,
        y1: 1,
    };
    c.insert(key(0, 1, 0), (), Stats::default(), None);
    assert_eq!(
        wanted(&c, r, 7, PAGE, 0).len(),
        2,
        "outer tiles never asked for"
    );
}

#[test]
fn a_cache_full_of_drawn_fallback_tiles_makes_room_for_the_planned_level() {
    // The browser budget, full of finer tiles from before a zoom out, all on screen now.
    let mut c = Cache::new(126);
    let visible = Rect::new(0.0, 0.0, 2048.0, 1536.0);
    let finer = Range::covering(4, visible).by_distance(7, PAGE);
    for k in finer.into_iter().take(126) {
        c.insert(k, (), Stats::default(), None);
    }
    let (range, rounds) = settle(&mut c, 0, visible, 8);
    assert!(
        rounds.is_some(),
        "planned tiles kept being evicted: requests never stop"
    );
    assert!(wanted(&c, range, 7, PAGE, 0).is_empty());
}

#[test]
fn covered_fallback_tiles_go_before_uncovered_ones_and_planned_tiles_stay() {
    let mut c = Cache::new(7);
    // Planned: level 0 tiles x 0..3. Fallback: a coarse level -4 tile (2048 units, over all
    // three) and finer level 2 tiles x 0..4 (under planned x 0 and 1).
    let plan = Range {
        level: 0,
        x0: 0,
        y0: 0,
        x1: 3,
        y1: 1,
    };
    c.insert(key(-4, 0, 0), (), Stats::default(), None);
    for x in 0..4 {
        c.insert(key(2, x, 0), (), Stats::default(), None);
    }
    c.insert(key(0, 0, 0), (), Stats::default(), None);
    let mut other_page = key(0, 1, 0);
    other_page.page = Guid(0, 2);
    c.insert(other_page, (), Stats::default(), None);
    c.begin_frame(7, PAGE, plan);
    for k in c.draw_order(7, PAGE, 0, Rect::new(0.0, 0.0, 1536.0, 512.0)) {
        c.touch(&k);
    }
    // Planned tile 1 arrives: the off-screen tile of another page goes first.
    c.insert(key(0, 1, 0), (), Stats::default(), None);
    assert!(
        c.get(&other_page).is_none(),
        "the off-screen tile should go first"
    );
    // Planned tile 2 arrives: the finer tiles are covered by planned tiles 0 and 1, the
    // coarse one is not (tile 2 was missing when it was chosen), so the oldest finer goes.
    c.insert(key(0, 2, 0), (), Stats::default(), None);
    assert!(
        c.get(&key(2, 0, 0)).is_none(),
        "the oldest covered fallback should go"
    );
    assert!(
        c.get(&key(-4, 0, 0)).is_some(),
        "the uncovered fallback went first"
    );
    assert!(
        (0..3).all(|x| c.get(&key(0, x, 0)).is_some()),
        "a planned tile went"
    );
}
