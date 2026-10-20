//! A tiny font built in the test: the table shape of a bitmap-only color emoji face (cmap,
//! metrics and names, but no glyf, CFF or CFF2), so no real emoji font has to be shipped.

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend(x.to_be_bytes());
}

fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend(x.to_be_bytes());
}

/// A face named "Bitmap" with 1000 units per em that maps U+0020 to glyph 1 with
/// `space_advance`, and has no outline tables.
pub(crate) fn bitmap_only(space_advance: u16) -> Vec<u8> {
    let mut head = Vec::new();
    be32(&mut head, 0x0001_0000); // version
    be32(&mut head, 0); // fontRevision
    be32(&mut head, 0); // checksumAdjustment
    be32(&mut head, 0x5F0F_3CF5); // magic
    be16(&mut head, 0); // flags
    be16(&mut head, 1000); // unitsPerEm
    head.extend([0; 16]); // created, modified
    for v in [0, 0, 0, 0, 0, 8, 2, 0, 0] {
        be16(&mut head, v); // bbox, macStyle, lowestRecPPEM, direction, indexToLoc, glyphData
    }

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    be16(&mut hhea, 800); // ascender
    be16(&mut hhea, 200u16.wrapping_neg()); // descender -200
    be16(&mut hhea, 0); // lineGap
    be16(&mut hhea, space_advance); // advanceWidthMax
    for _ in 0..11 {
        be16(&mut hhea, 0);
    }
    be16(&mut hhea, 2); // numberOfHMetrics

    let mut hmtx = Vec::new();
    for _ in 0..2 {
        be16(&mut hmtx, space_advance);
        be16(&mut hmtx, 0);
    }

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0000_5000); // version 0.5, no outline limits
    be16(&mut maxp, 2); // numGlyphs

    // cmap: one format 4 subtable, U+0020 -> glyph 1, then the 0xFFFF terminator segment.
    let mut sub = Vec::new();
    for v in [4, 32, 0, 4, 4, 1, 0] {
        be16(&mut sub, v); // format, length, language, segCountX2, search fields
    }
    for v in [
        0x20,
        0xFFFF,
        0,
        0x20,
        0xFFFF,
        1u16.wrapping_sub(0x20),
        1,
        0,
        0,
    ] {
        be16(&mut sub, v); // endCodes, pad, startCodes, idDeltas, idRangeOffsets
    }
    let mut cmap = Vec::new();
    for v in [0, 1, 3, 1] {
        be16(&mut cmap, v); // version, numTables, Windows Unicode BMP
    }
    be32(&mut cmap, 12);
    cmap.extend(sub);

    // name: family (ID 1) and PostScript name (ID 6); fontdb skips a face without both.
    let family: Vec<u8> = "Bitmap".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut name = Vec::new();
    for v in [0, 2, 6 + 2 * 12] {
        be16(&mut name, v);
    }
    for id in [1, 6] {
        for v in [3, 1, 0x409, id, family.len() as u16, 0] {
            be16(&mut name, v);
        }
    }
    name.extend(family);

    let tables: [(&[u8; 4], Vec<u8>); 6] = [
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name),
    ];
    let mut out = Vec::new();
    be32(&mut out, 0x0001_0000);
    for v in [tables.len() as u16, 64, 2, 32] {
        be16(&mut out, v); // numTables, searchRange, entrySelector, rangeShift
    }
    let start = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in &tables {
        out.extend(*tag);
        be32(&mut out, 0); // checksum (not checked by the parsers)
        be32(&mut out, (start + body.len()) as u32);
        be32(&mut out, data.len() as u32);
        body.extend(data);
        body.resize(body.len().next_multiple_of(4), 0);
    }
    out.extend(body);
    out
}
