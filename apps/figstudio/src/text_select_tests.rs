use super::*;
use figstudio_fig::{Document, Obj};

fn fixture(name: &str) -> Document {
    Document::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures")
            .join(name),
    )
    .expect("public fixture")
}

#[test]
fn stored_fixture_hits_characters_and_line_edges() {
    let doc = fixture("noise_and_grain.fig");
    let data = &doc.node(Guid(1, 5)).expect("public text").data;
    let stored = data
        .path(&["derivedTextData", "glyphs"])
        .expect("stored layout")
        .items();
    for (i, ch) in [(0, 'T'), (2, 'x'), (4, ' '), (6, 'i'), (10, 'N'), (14, 'e')] {
        let p = stored[i].get("position").expect("position");
        let x = f64::from(p.num("x").expect("x")) + 0.1;
        let y = f64::from(p.num("y").expect("y")) - 1.0;
        assert_eq!(char_at(data, x, y), Some((i, ch)));
    }
    let y = f64::from(
        stored[0]
            .get("position")
            .expect("position")
            .num("y")
            .expect("y"),
    ) - 1.0;
    assert_eq!(char_at(data, -100.0, y), Some((0, 'T')));
    assert_eq!(char_at(data, 10000.0, y), Some((10, '\0')));
    let last_y = f64::from(
        stored[14]
            .get("position")
            .expect("position")
            .num("y")
            .expect("y"),
    ) - 1.0;
    assert_eq!(char_at(data, 10000.0, last_y), Some((15, '\0')));
    assert_eq!(range_rects(data, 0, 4).expect("layout").len(), 1);
    assert_eq!(range_rects(data, 15, 0).expect("wrapped layout").len(), 2);
}

#[test]
fn missing_or_invalid_glyphs_use_fallback_without_panics() {
    let doc = fixture("structure.fig");
    let data = &doc.node(Guid(1, 42)).expect("public text").data;
    assert_eq!(char_at(data, 0.0, 0.0), None);
    assert_eq!(range_rects(data, 0, 3), None);
    assert_eq!(char_at(&Val::Obj(Obj::new()), f64::NAN, 0.0), None);
}

#[test]
fn ranges_are_reversible_clamped_utf8_byte_offsets() {
    for (a, b, expected) in [
        (1, 6, "é猫"),
        (6, 1, "é猫"),
        (2, 5, "é"),
        (0, usize::MAX, "Aé猫Z"),
        (3, 3, ""),
    ] {
        assert_eq!(range_text("Aé猫Z", a, b), expected);
    }
}
