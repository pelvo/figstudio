//! Synthetic embedded-bitmap tables: every size record has its own format-17 PNG.
use ttf_parser::{Face, Tag};

pub(super) fn font(tags: (&[u8; 4], &[u8; 4]), pngs: [&[u8]; 2], ppems: [u8; 2]) -> Vec<u8> {
    let base = crate::text_test_font::colour_emoji(b"");
    let face = Face::parse(&base, 0).unwrap();
    let mut tables: Vec<_> = face
        .raw_face()
        .table_records
        .into_iter()
        .filter(|r| r.tag != Tag::from_bytes(b"sbix"))
        .map(|r| {
            (
                r.tag.to_bytes(),
                face.raw_face().table(r.tag).unwrap().to_vec(),
            )
        })
        .collect();
    let mut locations = Vec::new();
    let mut data = Vec::new();
    let version: u32 = if tags.0 == b"CBDT" {
        0x0003_0000
    } else {
        0x0002_0000
    };
    data.extend(version.to_be_bytes());
    locations.extend(version.to_be_bytes());
    locations.extend(2u32.to_be_bytes());
    for (i, ppem) in ppems.into_iter().enumerate() {
        // BitmapSize (48 bytes); one IndexSubTableArray entry per size.
        for value in [104 + i as u32 * 24, 24, 1, 0] {
            locations.extend(value.to_be_bytes());
        }
        locations.extend([0; 24]); // horizontal and vertical SbitLineMetrics
        locations.extend(1u16.to_be_bytes());
        locations.extend(1u16.to_be_bytes());
        locations.extend([ppem, ppem, 32, 1]);
    }
    for png in pngs {
        // Array entry, then index format 1 with start/end data offsets.
        locations.extend(1u16.to_be_bytes());
        locations.extend(1u16.to_be_bytes());
        locations.extend(8u32.to_be_bytes());
        locations.extend(1u16.to_be_bytes());
        locations.extend(17u16.to_be_bytes());
        locations.extend((data.len() as u32).to_be_bytes());
        locations.extend(0u32.to_be_bytes());
        locations.extend((9 + png.len() as u32).to_be_bytes());
        data.extend([2, 2, 1, 3, 2]); // SmallGlyphMetrics: h, w, bearing x/y, advance
        data.extend((png.len() as u32).to_be_bytes());
        data.extend(png);
    }
    tables.push((*tags.0, data));
    tables.push((*tags.1, locations));
    crate::text_test_font::sfnt(
        tables
            .iter()
            .map(|(tag, data)| (tag, data.clone()))
            .collect(),
    )
}
