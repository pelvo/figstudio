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
    font(space_advance, "Bitmap", 0x20, None, false)
}

/// Two PNG strikes of a synthetic emoji, to exercise colour decoding without installed fonts.
pub(crate) fn colour_emoji(png: &[u8]) -> Vec<u8> {
    emoji_strike(png, (0, 0), b"png ", false)
}

/// A real outline family, not a colour-family alias.
pub(crate) fn outline_emoji() -> Vec<u8> {
    font(1000, "Noto Emoji", 0x2764, None, true)
}

pub(crate) fn emoji_strike(
    png: &[u8],
    origin: (i16, i16),
    tag: &[u8; 4],
    outline: bool,
) -> Vec<u8> {
    let mut sbix = Vec::new();
    for v in [1, 1] {
        be16(&mut sbix, v);
    }
    for v in [2, 16, (16 + 24 + png.len()) as u32] {
        be32(&mut sbix, v);
    }
    for ppem in [16, 64] {
        be16(&mut sbix, ppem);
        be16(&mut sbix, 72);
        for offset in [16, 16, (24 + png.len()) as u32] {
            be32(&mut sbix, offset);
        }
        sbix.extend(origin.0.to_be_bytes());
        sbix.extend(origin.1.to_be_bytes());
        sbix.extend(tag);
        sbix.extend(png);
    }
    font(1000, "Noto Color Emoji", 0x2764, Some(sbix), outline)
}

fn font(
    space_advance: u16,
    family: &str,
    codepoint: u16,
    sbix: Option<Vec<u8>>,
    outline: bool,
) -> Vec<u8> {
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
        codepoint,
        0xFFFF,
        0,
        codepoint,
        0xFFFF,
        1u16.wrapping_sub(codepoint),
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
    let family: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
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

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"maxp", maxp),
        (b"name", name),
    ];
    if outline {
        // One rectangular contour, glyph 0 empty, glyph 1 from (0, 0) to (800, 800).
        let mut glyf = Vec::new();
        for n in [1, 0, 0, 800, 800, 3, 0] {
            be16(&mut glyf, n);
        }
        glyf.extend([1; 4]); // on-curve, signed 16-bit x/y deltas
        for n in [0i16, 800, 0, -800, 0, 0, 800, 0] {
            glyf.extend(n.to_be_bytes());
        }
        let mut loca = Vec::new();
        for n in [0, 0, glyf.len() as u16 / 2] {
            be16(&mut loca, n);
        }
        tables.push((b"glyf", glyf));
        tables.push((b"loca", loca));
    }
    if let Some(sbix) = sbix {
        tables.push((b"sbix", sbix));
    }
    tables.sort_by_key(|(tag, _)| *tag);
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
