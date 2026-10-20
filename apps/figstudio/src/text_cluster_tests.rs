//! Synthetic stored layouts exercise clusters absent from the public ASCII fixtures.
use super::*;
use figstudio_fig::Obj;

fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}

fn layout(text: &str, starts: &[u32]) -> Val {
    obj(&[
        ("textData", obj(&[("characters", Val::Str(text.into()))])),
        (
            "derivedTextData",
            obj(&[(
                "glyphs",
                Val::Arr(
                    starts
                        .iter()
                        .enumerate()
                        .map(|(i, start)| {
                            obj(&[
                                ("firstCharacter", Val::Int(i64::from(*start))),
                                ("fontSize", Val::Float(10.0)),
                                ("advance", Val::Float(0.8)),
                                (
                                    "position",
                                    obj(&[
                                        ("x", Val::Float(i as f64 * 8.0)),
                                        ("y", Val::Float(10.0)),
                                    ]),
                                ),
                            ])
                        })
                        .collect(),
                ),
            )]),
        ),
    ])
}

fn copied(data: &Val, from: f64, to: f64) -> &str {
    let a = char_at(data, from, 5.0).unwrap().0;
    let b = char_at(data, to, 5.0).unwrap().0;
    range_text(characters(data), a, b)
}

#[test]
fn hard_break_without_a_glyph_does_not_shift_later_text() {
    let mut data = layout("a\nbc", &[0, 2, 3]);
    let glyphs = data
        .get_mut("derivedTextData")
        .unwrap()
        .get_mut("glyphs")
        .unwrap();
    if let Val::Arr(glyphs) = glyphs {
        for g in &mut glyphs[1..] {
            g.get_mut("position").unwrap().set("y", Val::Float(30.0));
        }
    }
    let a = char_at(&data, 8.1, 25.0).unwrap();
    let b = char_at(&data, 100.0, 25.0).unwrap();
    assert_eq!(a, (2, 'b'));
    assert_eq!(range_text(characters(&data), a.0, b.0), "bc");
    assert_eq!(
        range_rects(&data, 2, 4).unwrap(),
        vec![Rect::new(8.0, 20.0, 16.0 + 10.0 * f64::from(0.8_f32), 32.0)]
    );
}

#[test]
fn ligature_cluster_covers_both_characters_and_partial_ranges_highlight_it() {
    let data = layout("fix", &[0, 2]);
    assert_eq!(copied(&data, 0.1, 8.1), "fi");
    assert_eq!(copied(&data, 8.1, 100.0), "x");
    assert_eq!(
        range_rects(&data, 1, 2).unwrap(),
        vec![Rect::new(0.0, 0.0, 10.0 * f64::from(0.8_f32), 12.0)]
    );
}

#[test]
fn scalar_clusters_map_multibyte_and_supplementary_characters_to_utf8() {
    let data = layout("é😀fiZ", &[0, 1, 2, 4]);
    assert_eq!(copied(&data, 0.1, 8.1), "é");
    assert_eq!(copied(&data, 8.1, 16.1), "😀");
    assert_eq!(char_at(&data, 16.1, 5.0), Some((6, 'f')));
    assert_eq!(copied(&data, 16.1, 24.1), "fi");
    assert_eq!(copied(&data, 24.1, 100.0), "Z");
}

#[test]
fn cluster_midpoints_snap_to_whole_utf8_boundaries() {
    let data = layout("é😀fiZ", &[0, 1, 2, 4]);
    for (x, boundary) in [(10.0, 2), (14.0, 6), (18.0, 6), (22.0, 8)] {
        let index = char_at(&data, x, 5.0).unwrap().0;
        assert_eq!(index, boundary, "x={x}");
        assert!(characters(&data).is_char_boundary(index));
    }
    assert_eq!(copied(&data, 18.0, 22.0), "fi");
    assert_eq!(copied(&data, 22.0, 18.0), "fi");
}

#[test]
fn truncation_ellipsis_and_hidden_glyphs_are_not_selectable() {
    let mut data = layout("abcdef", &[0, 1, 2, 2, 3, 4, 5]);
    // Two visible letters, a sourceless ellipsis, then stored but hidden glyphs.
    if let Val::Arr(glyphs) = data
        .get_mut("derivedTextData")
        .unwrap()
        .get_mut("glyphs")
        .unwrap()
    {
        glyphs[2] = obj(&[
            ("fontSize", Val::Float(10.0)),
            ("advance", Val::Float(0.8)),
            (
                "position",
                obj(&[("x", Val::Float(16.0)), ("y", Val::Float(10.0))]),
            ),
        ]);
    }
    data.get_mut("derivedTextData")
        .unwrap()
        .set("truncationStartIndex", Val::Int(3));
    assert_eq!(copied(&data, 0.1, 100.0), "ab");
    assert_eq!(char_at(&data, 17.0, 5.0), Some((2, '\0')));
    assert!(range_rects(&data, 2, 6).unwrap().is_empty());
    data.get_mut("derivedTextData")
        .unwrap()
        .set("truncationStartIndex", Val::Int(-1));
    assert_eq!(copied(&data, 0.1, 100.0), "abcdef");
    data.get_mut("derivedTextData")
        .unwrap()
        .set("truncationStartIndex", Val::Int(0));
    assert_eq!(char_at(&data, 0.1, 5.0), None);
    assert_eq!(range_rects(&data, 0, 6), Some(Vec::new()));
    data.get_mut("derivedTextData")
        .unwrap()
        .set("truncationStartIndex", Val::Int(i64::from(u32::MAX)));
    assert_eq!(copied(&data, 0.1, 100.0), "abcdef");
}

#[test]
fn stored_advances_define_the_last_glyph_and_trailing_space_edges() {
    let doc = super::tests::fixture("noise_and_grain.fig");
    let data = &doc.node(Guid(1, 5)).unwrap().data;
    let g = &data.path(&["derivedTextData", "glyphs"]).unwrap().items()[14];
    let p = g.get("position").unwrap();
    let right =
        f64::from(p.num("x").unwrap() + g.num("fontSize").unwrap() * g.num("advance").unwrap());
    assert_eq!(
        char_at(data, right - 0.01, f64::from(p.num("y").unwrap()) - 1.0),
        Some((15, '\0'))
    );
    let wide = layout("w", &[0]);
    assert_eq!(char_at(&wide, 7.9, 5.0), Some((1, '\0')));
    assert_eq!(copied(&wide, 0.1, 8.1), "w");
    let mut data = layout("w ", &[0, 1]);
    if let Val::Arr(glyphs) = data
        .get_mut("derivedTextData")
        .unwrap()
        .get_mut("glyphs")
        .unwrap()
    {
        glyphs[1].set("advance", Val::Float(0.2));
    }
    assert_eq!(char_at(&data, 10.1, 5.0), Some((2, '\0')));
    assert_eq!(copied(&data, 0.1, 10.1), "w ");
}

#[test]
fn repeated_and_reordered_cluster_starts_keep_logical_boundaries() {
    let data = layout("aébc", &[2, 0, 0, 3]);
    assert_eq!(copied(&data, 0.1, 24.1), "b");
    assert_eq!(char_at(&data, 8.1, 5.0), Some((0, 'a')));
    assert_eq!(char_at(&data, 16.1, 5.0), Some((0, 'a')));
    assert_eq!(range_rects(&data, 1, 3).unwrap().len(), 1);
}

#[test]
fn missing_or_invalid_advances_fall_back_to_neighbor_then_em_width() {
    for advance in [
        Val::Float(f64::NAN),
        Val::Float(-1.0),
        Val::Str("missing".into()),
    ] {
        let mut data = layout("ab", &[0, 1]);
        if let Val::Arr(glyphs) = data
            .get_mut("derivedTextData")
            .unwrap()
            .get_mut("glyphs")
            .unwrap()
        {
            glyphs[0].set("advance", advance.clone());
            glyphs[1].set("advance", advance);
        }
        assert_eq!(char_at(&data, 7.0, 5.0), Some((1, '\0')));
        assert_eq!(char_at(&data, 13.9, 5.0), Some((2, '\0')));
        assert_eq!(char_at(&data, 14.1, 5.0), Some((2, '\0')));
    }
}

#[test]
fn malformed_cluster_indices_never_split_utf8_or_panic() {
    let data = layout("😀Z", &[1, u32::MAX]);
    if let Some((index, _)) = char_at(&data, 0.1, 5.0) {
        assert!(characters(&data).is_char_boundary(index));
        let _ = range_text(characters(&data), index, usize::MAX);
    }
    let _ = range_rects(&data, 0, usize::MAX);
}
