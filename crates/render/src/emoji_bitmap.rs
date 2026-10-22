//! Fontations exposes size records directly; ttf-parser only exposes a best-size selector.
use read_fonts::{
    FontData, FontRead,
    tables::{
        bitmap::{BitmapContent, BitmapDataFormat, BitmapMetrics},
        cbdt::Cbdt,
        cblc::Cblc,
    },
    types::GlyphId,
};
use ttf_parser::{Face, RasterGlyphImage, RasterImageFormat, Tag};

pub(crate) fn strikes<'a>(
    face: &Face<'a>,
    glyph: ttf_parser::GlyphId,
) -> Vec<RasterGlyphImage<'a>> {
    let mut out = Vec::new();
    // These table pairs share the embedded-bitmap layout, as in ttf-parser's cbdt reader.
    for (location_tag, data_tag) in [(b"CBLC", b"CBDT"), (b"bloc", b"bdat"), (b"EBLC", b"EBDT")] {
        let Some(locations) = face.raw_face().table(Tag::from_bytes(location_tag)) else {
            continue;
        };
        let Some(data) = face.raw_face().table(Tag::from_bytes(data_tag)) else {
            continue;
        };
        let Ok(locations) = Cblc::read(FontData::new(locations)) else {
            continue;
        };
        let Ok(data) = Cbdt::read(FontData::new(data)) else {
            continue;
        };
        for size in locations.bitmap_sizes() {
            let Ok(location) =
                size.location(locations.offset_data(), GlyphId::new(u32::from(glyph.0)))
            else {
                continue;
            };
            let Ok(bitmap) = data.data(&location) else {
                continue;
            };
            let BitmapContent::Data(BitmapDataFormat::Png, png) = bitmap.content else {
                continue;
            };
            let (width, height, x, y) = match bitmap.metrics {
                BitmapMetrics::Small(m) => (m.width(), m.height(), m.bearing_x(), m.bearing_y()),
                BitmapMetrics::Big(m) => (
                    m.width(),
                    m.height(),
                    m.hori_bearing_x(),
                    m.hori_bearing_y(),
                ),
            };
            out.push(RasterGlyphImage {
                x: i16::from(x),
                y: i16::from(y) - i16::from(height),
                width: u16::from(width),
                height: u16::from(height),
                pixels_per_em: u16::from(size.ppem_x()),
                format: RasterImageFormat::PNG,
                data: png,
            });
        }
    }
    out
}
