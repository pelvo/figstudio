use super::*;

#[test]
fn supplied_face_bytes_are_shared_across_renderers() {
    let bytes = Arc::new(epaint_default_fonts::HACK_REGULAR.to_vec());
    let mut renderers = Vec::new();
    for _ in 0..4 {
        let mut fonts = SystemFonts {
            db: Some(Database::new()),
            ..SystemFonts::default()
        };
        fonts.add_font_data(bytes.clone(), 0);
        let id = fonts.supplied[0];
        let (face, index) = fonts.face_data(id).unwrap();
        assert_eq!(index, 0);
        assert!(
            Arc::ptr_eq(&bytes, &face),
            "face_data copied supplied bytes"
        );
        renderers.push(fonts);
    }
    eprintln!(
        "4 renderers share {} source bytes; retained references={}",
        bytes.len(),
        Arc::strong_count(&bytes)
    );
}

fn collection() -> Arc<Vec<u8>> {
    let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02\0\0\0\0\0\0\0\0".to_vec();
    for (i, font) in [
        epaint_default_fonts::HACK_REGULAR,
        epaint_default_fonts::UBUNTU_LIGHT,
    ]
    .into_iter()
    .enumerate()
    {
        let base = bytes.len();
        bytes[12 + i * 4..16 + i * 4].copy_from_slice(&(base as u32).to_be_bytes());
        let mut face = font.to_vec();
        let tables = u16::from_be_bytes(face[4..6].try_into().unwrap());
        for table in 0..usize::from(tables) {
            let at = 12 + table * 16 + 8;
            let offset = u32::from_be_bytes(face[at..at + 4].try_into().unwrap());
            face[at..at + 4].copy_from_slice(&(offset + base as u32).to_be_bytes());
        }
        bytes.extend(face);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
    }
    Arc::new(bytes)
}

#[test]
fn supplied_collection_uses_only_the_selected_face_and_its_source() {
    let bytes = collection();
    let other = Arc::new(epaint_default_fonts::NOTO_EMOJI_REGULAR.to_vec());
    for index in [0, 1, 2] {
        let mut fonts = SystemFonts {
            db: Some(Database::new()),
            ..SystemFonts::default()
        };
        fonts.add_font_data(other.clone(), 0);
        let other_id = fonts.supplied[0];
        fonts.add_font_data(bytes.clone(), index);
        assert!(Arc::ptr_eq(&fonts.face_data(other_id).unwrap().0, &other));
        if index == 2 {
            assert_eq!(
                fonts.supplied.len(),
                1,
                "invalid face must not choose face zero"
            );
        } else {
            assert_eq!(fonts.supplied.len(), 2);
            let id = fonts.supplied[1];
            assert_eq!(fonts.db().faces().count(), 2);
            let (data, actual_index) = fonts.face_data(id).unwrap();
            assert!(Arc::ptr_eq(&data, &bytes));
            assert_eq!(actual_index, index);
            assert!(
                Face::parse(&data, actual_index)
                    .unwrap()
                    .glyph_index('A')
                    .is_some()
            );
        }
    }
}

/// The vertical distance between the two lines of "A\nA" in a font family no supplied face has.
fn line_pitch(supplied: &[&'static [u8]]) -> f64 {
    let mut fonts = SystemFonts {
        db: Some(Database::new()),
        ..SystemFonts::default()
    };
    for bytes in supplied {
        fonts.add_font_data(Arc::new(bytes.to_vec()), 0);
    }
    let object = |pairs: &[(&str, Val)]| {
        let mut object = Val::Obj(Default::default());
        for (key, value) in pairs {
            object.set(key, value.clone());
        }
        object
    };
    let data = object(&[
        ("fontSize", Val::Float(20.0)),
        (
            "textData",
            object(&[("characters", Val::Str("A\nA".into()))]),
        ),
        (
            "fontName",
            object(&[("family", Val::Str("Unavailable family".into()))]),
        ),
    ]);
    let paths = fonts.layout(&data).unwrap();
    let tops: Vec<f64> = paths
        .iter()
        .map(|path| vello_cpu::kurbo::Shape::bounding_box(path).y0)
        .collect();
    assert_eq!(tops.len(), 2);
    tops[1] - tops[0]
}

#[test]
fn supplied_fonts_keep_their_order_as_fallbacks_without_installed_fonts() {
    let alone = line_pitch(&[epaint_default_fonts::UBUNTU_LIGHT]);
    let with_emoji = line_pitch(&[
        epaint_default_fonts::UBUNTU_LIGHT,
        epaint_default_fonts::NOTO_EMOJI_REGULAR,
    ]);
    assert!(
        (alone - with_emoji).abs() < 1e-9,
        "a later supplied emoji face set the line height: {alone} vs {with_emoji}"
    );
}
