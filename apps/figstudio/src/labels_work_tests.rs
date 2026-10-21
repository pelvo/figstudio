//! Deterministic work and broad-phase equivalence checks, not wall-clock thresholds.
use super::tests::{document, node};
use super::*;

#[test]
fn painting_and_find_rects_reuse_the_candidate_layout() {
    let doc = document(vec![
        node(1, 0, "CANVAS", "page", 0.0, 0.0),
        node(2, 1, "FRAME", "Label", 0.0, 300.0),
    ]);
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        SHAPED.set(0);
        let ls = labels(
            ui.painter(),
            &doc,
            Guid(9, 1),
            View {
                pan: Vec2::new(0.0, 100.0),
                zoom: 1.0,
            },
            Pos2::ZERO,
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0)),
        );
        assert_eq!(ls.len(), 1);
        assert_eq!(SHAPED.get(), 1);
        paint(ui.painter(), &ls);
        assert!(rect(ui.painter(), &ls[0]).is_positive());
        assert_eq!(SHAPED.get(), 1, "paint/find must not shape the label again");
    });
    out.textures_delta.clear();
}

#[test]
fn spatial_queries_match_a_linear_scan_including_wide_and_negative_bounds() {
    let mut grid = spatial::Grid::default();
    let mut items = Vec::new();
    for i in 0..300 {
        let x = (i % 20) as f32 * 180.0 - 2000.0;
        let y = (i / 20) as f32 * 90.0 - 800.0;
        let r = Rect::from_min_size(
            Pos2::new(x, y),
            Vec2::new(if i % 41 == 0 { 100_000.0 } else { 150.0 }, 70.0),
        );
        grid.insert(r, i);
        items.push((r, i));
    }
    for i in 0..500 {
        let query = Rect::from_min_size(
            Pos2::new(
                (i % 29) as f32 * 170.0 - 2500.0,
                (i / 29) as f32 * 100.0 - 1000.0,
            ),
            Vec2::new(if i % 59 == 0 { 200_000.0 } else { 220.0 }, 14.0),
        );
        let expected = items
            .iter()
            .any(|(r, id)| query.intersect(*r).is_positive() && id % 3 == 0);
        assert_eq!(grid.any(query, |id| id % 3 == 0), expected, "{i}");
    }
}

#[test]
fn distant_frames_do_not_reach_the_overlap_predicate() {
    let mut grid = spatial::Grid::default();
    for i in 0..5000 {
        grid.insert(
            Rect::from_min_size(Pos2::new(i as f32 * 800.0, 0.0), Vec2::new(600.0, 50.0)),
            i,
        );
    }
    let mut checked = 0;
    let query = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(150.0, 14.0));
    assert!(!grid.any(query, |_| {
        checked += 1;
        false
    }));
    assert_eq!(checked, 1, "only the nearby frame can intersect the label");
}
