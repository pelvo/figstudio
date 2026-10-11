use super::*;
use figstudio_render::Stats;

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

#[test]
fn the_level_is_the_first_at_least_as_sharp_as_the_screen() {
    assert_eq!(level_for(1.0), 0);
    assert_eq!(level_for(2.0), 2);
    // 100 % on a 1.5x screen: level 2 (2 px per unit), scaled down by 0.75.
    assert_eq!(level_for(1.5), 2);
    assert_eq!(level_for(1.3), 1);
    assert_eq!(level_for(0.5), -2);
    assert_eq!(level_for(0.3), -3);
    for z in [0.011_f32, 0.07, 0.9, 1.0, 1.25, 3.3, 17.0, 300.0] {
        let s = scale(level_for(z));
        // Never blurrier than the screen, never more than one half-step sharper.
        assert!(s >= f64::from(z) * 0.999_99, "{z}: {s}");
        assert!(
            s < f64::from(z) * std::f64::consts::SQRT_2 * 1.000_01,
            "{z}: {s}"
        );
    }
    assert_eq!(level_for(0.0), -40);
    assert_eq!(level_for(f32::INFINITY), 40);
}

#[test]
fn a_tile_transform_maps_its_page_rect_onto_its_pixels() {
    for k in [key(0, 0, 0), key(3, -2, 5), key(-5, 1, -1)] {
        let r = k.page_rect();
        let t = k.transform();
        let (a, b) = (t * r.origin(), t * kurbo_point(r.x1, r.y1));
        let g = f64::from(GUTTER);
        assert!(
            (a.x - g).abs() < 1e-6 && (a.y - g).abs() < 1e-6,
            "{k:?}: {a:?}"
        );
        let end = f64::from(TILE) + g;
        assert!(
            (b.x - end).abs() < 1e-6 && (b.y - end).abs() < 1e-6,
            "{k:?}: {b:?}"
        );
    }
}

fn kurbo_point(x: f64, y: f64) -> figstudio_render::kurbo::Point {
    figstudio_render::kurbo::Point::new(x, y)
}

#[test]
fn the_visible_tiles_cover_negative_coordinates_and_fractional_screens() {
    // Level 0: 512 page units per tile. The view shows page x -100..412, y -600..-88.
    let v = visible_page(1.0, [100.0, 600.0], [512.0, 512.0]);
    assert_eq!(
        Range::covering(0, v),
        Range {
            level: 0,
            x0: -1,
            y0: -2,
            x1: 1,
            y1: 0
        }
    );
    // 1.5 ppp, zoom 1: level 2 draws 2 px per unit, 256 units per tile; an 800x600 point
    // canvas at pan (0, 0) shows units 0..800 x 0..600 -> tiles 0..4 x 0..3.
    let level = level_for(1.0 * 1.5);
    let r = Range::covering(level, visible_page(1.0, [0.0, 0.0], [800.0, 600.0]));
    assert_eq!(
        r,
        Range {
            level: 2,
            x0: 0,
            y0: 0,
            x1: 4,
            y1: 3
        }
    );
    assert_eq!(
        r.grow(1),
        Range {
            level: 2,
            x0: -1,
            y0: -1,
            x1: 5,
            y1: 4
        }
    );
}

#[test]
fn absurd_views_give_no_panic() {
    let r = Range::covering(40, visible_page(256.0, [f32::MAX, -f32::MAX], [1e9, 1e9]));
    assert!(r.by_distance(7, PAGE).len() <= 64 * 64);
    let huge = Rect::new(-1e12, -1e12, 1e12, 1e12);
    assert!(Range::covering(0, huge).grow(1).by_distance(7, PAGE).len() <= 66 * 66);
}

#[test]
fn requests_go_nearest_the_centre_first() {
    let r = Range {
        level: 0,
        x0: -2,
        y0: 0,
        x1: 3,
        y1: 1,
    };
    let xs: Vec<i32> = r.by_distance(7, PAGE).iter().map(|k| k.x).collect();
    assert_eq!(xs[0], 0);
    assert_eq!(xs.len(), 5);
    assert!(xs[1..3].contains(&-1) && xs[1..3].contains(&1));
    assert!(xs[3..].contains(&-2) && xs[3..].contains(&2));
}

#[test]
fn the_cache_evicts_the_least_recently_used_tile_over_its_budget() {
    let mut c = Cache::new(2);
    c.insert(key(0, 0, 0), "a", Stats::default(), None);
    c.insert(key(0, 1, 0), "b", Stats::default(), None);
    c.touch(&key(0, 0, 0));
    c.insert(key(0, 2, 0), "c", Stats::default(), None);
    assert_eq!(c.len(), 2);
    assert!(c.get(&key(0, 0, 0)).is_some(), "the used tile was evicted");
    assert!(c.get(&key(0, 1, 0)).is_none(), "the oldest tile stayed");
    assert!(c.get(&key(0, 2, 0)).is_some());
}

#[test]
fn a_missing_tile_is_covered_by_coarser_and_finer_tiles_drawn_under_the_sharp_ones() {
    let mut c = Cache::new(16);
    // Level 2 is sharp; tile (1, 0) of level 2 is missing.
    c.insert(key(2, 0, 0), (), Stats::default(), None);
    c.insert(key(0, 0, 0), (), Stats::default(), None); // coarser, covers 0..512
    c.insert(key(4, 2, 0), (), Stats::default(), None); // finer, covers 256..384
    c.insert(key(4, 40, 40), (), Stats::default(), None); // finer, out of view
    let mut other_page = key(0, 0, 0);
    other_page.page = Guid(0, 2);
    c.insert(other_page, (), Stats::default(), None);
    let missing = key(2, 1, 0).page_rect();
    let order = c.draw_order(7, PAGE, 2, missing.union(key(2, 0, 0).page_rect()));
    assert_eq!(order, vec![key(0, 0, 0), key(4, 2, 0), key(2, 0, 0)]);
    // What is drawn under the missing tile: the coarse one and the finer one.
    let under: Vec<Key> = order
        .into_iter()
        .filter(|k| k.page_rect().intersect(missing).area() > 0.0)
        .collect();
    assert_eq!(under, vec![key(0, 0, 0), key(4, 2, 0)]);
}

#[test]
fn only_missing_or_image_stale_tiles_are_wanted() {
    let mut c = Cache::new(16);
    let r = Range {
        level: 0,
        x0: 0,
        y0: 0,
        x1: 3,
        y1: 1,
    };
    c.insert(key(0, 1, 0), (), Stats::default(), None);
    c.insert(key(0, 2, 0), (), Stats::default(), Some(4));
    assert_eq!(wanted(&c, r, 7, PAGE, 4), vec![key(0, 0, 0)]);
    // Images arrived since tile (2, 0) was drawn with placeholders: draw it again.
    let w = wanted(&c, r, 7, PAGE, 5);
    assert_eq!(w.len(), 2);
    assert!(w.contains(&key(0, 2, 0)) && !w.contains(&key(0, 1, 0)));
    // Another document or page shares no tiles.
    assert_eq!(wanted(&c, r, 8, PAGE, 4).len(), 3);
    assert_eq!(wanted(&c, r, 7, Guid(0, 2), 4).len(), 3);
}

#[test]
fn a_batch_is_sent_only_when_the_wanted_tiles_change() {
    let pending = vec![key(0, 0, 0), key(0, 1, 0)];
    assert_eq!(next_batch(vec![key(0, 1, 0), key(0, 0, 0)], &pending), None);
    // The view moved: the new batch replaces the old, so tile (0, 0) is skipped.
    assert_eq!(
        next_batch(vec![key(0, 1, 0), key(0, 2, 0)], &pending),
        Some(vec![key(0, 1, 0), key(0, 2, 0)])
    );
    assert_eq!(next_batch(vec![], &[]), None);
    assert_eq!(next_batch(vec![], &pending), Some(vec![]));
}

#[test]
fn retain_drops_the_tiles_of_a_closed_document() {
    let mut c = Cache::new(16);
    c.insert(key(0, 0, 0), (), Stats::default(), None);
    let mut other = key(0, 0, 0);
    other.doc = 8;
    c.insert(other, (), Stats::default(), None);
    c.retain(|k| k.doc != 7);
    assert_eq!(c.len(), 1);
    assert!(c.get(&other).is_some());
}

#[test]
fn the_plan_never_lists_more_than_half_the_cache() {
    for budget in [8, 120, 240] {
        for (w, h) in [
            (300.0, 200.0),
            (1920.0, 1080.0),
            (7680.0, 4320.0),
            (16000.0, 9000.0),
        ] {
            for pan in [0.0, 37.3, 255.9, 511.0] {
                let visible = visible_page(1.0, [pan, -pan], [w, h]);
                let n = plan(0, visible, budget).by_distance(7, PAGE).len();
                assert!(n <= budget / 2, "{budget} {w}x{h} pan {pan}: {n}");
            }
        }
    }
}

#[test]
fn a_range_counts_its_tiles_without_listing_them() {
    // A coarse tile seen at a level 12 half-steps finer covers 64 x 64 tiles.
    let under = Range::covering(12, key(0, 0, 0).page_rect());
    assert_eq!(under.count(), 64 * 64);
    assert_eq!(under.count(), under.by_distance(7, PAGE).len() as u64);
    let empty = Range {
        level: 0,
        x0: 3,
        y0: 3,
        x1: 3,
        y1: 9,
    };
    assert_eq!(empty.count(), 0);
    let inverted = Range {
        level: 0,
        x0: 5,
        y0: 0,
        x1: 1,
        y1: 4,
    };
    assert_eq!(inverted.count(), 0);
}
