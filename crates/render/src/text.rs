//! Text: stored glyph outlines first, installed fonts as a fallback.

use figstudio_fig::Val;
use vello_cpu::kurbo::{Affine, BezPath};

/// Glyph outlines Figma stored with the text (`derivedTextData.glyphs`), in node space.
/// Empty when the file has none (older files, or text never laid out by the editor).
pub fn stored_glyphs(data: &Val, blobs: &[Vec<u8>]) -> Vec<BezPath> {
    let mut out = Vec::new();
    for glyph in visible_glyphs(data) {
        if is_emoji(glyph) {
            continue;
        }
        let Some(index) = glyph.get("commandsBlob").and_then(Val::as_u32) else {
            continue;
        };
        let Some(mut path) = blobs
            .get(index as usize)
            .and_then(|b| crate::geom::blob_path(b))
        else {
            continue;
        };
        let size = f64::from(glyph.num("fontSize").unwrap_or(12.0));
        let pos = glyph.get("position");
        let x = f64::from(pos.and_then(|p| p.num("x")).unwrap_or(0.0));
        let y = f64::from(pos.and_then(|p| p.num("y")).unwrap_or(0.0));
        // Outlines are in em units with y pointing up.
        path.apply_affine(Affine::translate((x, y)) * Affine::scale_non_uniform(size, -size));
        out.push(path);
    }
    out
}

pub(crate) fn visible_glyphs(data: &Val) -> &[Val] {
    let glyphs = data
        .path(&["derivedTextData", "glyphs"])
        .map(Val::items)
        .unwrap_or_default();
    // The visible run ends just past the stored ellipsis, including emoji markers.
    let visible = data
        .path(&["derivedTextData", "truncationStartIndex"])
        .and_then(Val::as_u32)
        .map_or(glyphs.len(), |i| glyphs.len().min(i as usize));
    &glyphs[..visible]
}

pub(crate) fn is_emoji(glyph: &Val) -> bool {
    glyph
        .get("emojiCodePoints")
        .is_some_and(|v| !v.items().is_empty())
}

pub fn characters(data: &Val) -> &str {
    data.path(&["textData", "characters"])
        .and_then(Val::as_str)
        .unwrap_or("")
}

pub use system::SystemFonts;
#[cfg(feature = "system-fonts")]
pub(crate) use system::{Builder, SYMBOL_FAMILIES};

mod system {
    #[cfg(all(test, feature = "system-fonts"))]
    mod cache_tests;
    #[cfg(test)]
    mod supplied_tests;
    use std::collections::HashMap;
    #[cfg(feature = "system-fonts")]
    use std::collections::VecDeque;
    use std::sync::Arc;

    use figstudio_fig::Val;
    use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};
    use ttf_parser::{Face, OutlineBuilder};
    use vello_cpu::kurbo::{Affine, BezPath};

    /// Families tried, in order, for characters the requested font lacks. Symbols and emoji
    /// need faces that carry them as outlines: `glyph_for` skips a face whose glyph does not
    /// outline (bitmap-only color emoji, empty glyphs). fontdb matches names exactly, so both
    /// spellings of Symbols 2 are listed.
    const FALLBACK_FAMILIES: &[&str] = &[
        "Inter",
        "Noto Sans",
        "Noto Sans CJK JP",
        "Noto Sans CJK TC",
        "Noto Sans CJK SC",
        "Source Han Sans",
        "Hiragino Sans",
        "PingFang TC",
    ];

    pub(crate) const SYMBOL_FAMILIES: &[&str] = &[
        "Noto Emoji",
        "Noto Sans Symbols",
        "Noto Sans Symbols 2",
        "Noto Sans Symbols2",
        "Noto Sans Math",
        "Segoe UI Symbol",
        "Apple Symbols",
        "Symbola",
        "DejaVu Sans",
        "Arial Unicode MS",
    ];

    /// Font file bytes and the face index inside it.
    type FaceData = (Arc<Vec<u8>>, u32);

    #[derive(Default)]
    pub struct SystemFonts {
        db: Option<Database>,
        faces: HashMap<ID, Option<FaceData>>,
        fallbacks: Option<Vec<ID>>,
        supplied: Vec<ID>,
        /// Whether `db` holds the installed fonts. Without them (the browser) the supplied
        /// faces are the fallbacks, in the order they were supplied.
        installed: bool,
        #[cfg(feature = "system-fonts")]
        emojis: HashMap<char, Option<crate::emoji_font::Resolved>>,
        #[cfg(feature = "system-fonts")]
        emoji_order: VecDeque<char>,
        #[cfg(feature = "system-fonts")]
        emoji_bytes: usize,
    }

    pub(crate) struct Builder(pub(crate) BezPath);

    impl OutlineBuilder for Builder {
        fn move_to(&mut self, x: f32, y: f32) {
            self.0.move_to((f64::from(x), f64::from(y)));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.0.line_to((f64::from(x), f64::from(y)));
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.0
                .quad_to((f64::from(x1), f64::from(y1)), (f64::from(x), f64::from(y)));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.0.curve_to(
                (f64::from(x1), f64::from(y1)),
                (f64::from(x2), f64::from(y2)),
                (f64::from(x), f64::from(y)),
            );
        }
        fn close(&mut self) {
            self.0.close_path();
        }
    }

    fn weight_of(style: &str) -> Weight {
        Weight(figstudio_fig::tokens::font_weight(style))
    }

    struct Placed {
        /// The glyph outline in font units, y up.
        path: BezPath,
        units_per_em: f64,
        x: f64,
        advance: f64,
        ch: char,
    }

    impl SystemFonts {
        /// Fonts from `db` instead of the installed ones.
        #[cfg(all(test, feature = "system-fonts"))]
        pub(crate) fn with_database(db: Database) -> Self {
            Self {
                db: Some(db),
                ..Self::default()
            }
        }

        pub fn add_font_data(&mut self, bytes: Arc<Vec<u8>>, index: u32) {
            self.db();
            if let Some(db) = &mut self.db {
                let ids = db.load_font_source(fontdb::Source::Binary(bytes.clone()));
                for id in ids {
                    if db.face(id).is_some_and(|face| face.index == index) {
                        self.supplied.push(id);
                        self.faces.insert(id, Some((bytes.clone(), index)));
                    } else {
                        db.remove_face(id);
                    }
                }
            }
            self.fallbacks = None;
        }

        #[cfg(not(feature = "system-fonts"))]
        pub(crate) fn emoji(&mut self, _ch: char, _pixels: u16) -> Option<crate::emoji::Glyph> {
            None
        }

        #[cfg(feature = "system-fonts")]
        pub(crate) fn emoji(&mut self, ch: char, pixels: u16) -> Option<crate::emoji::Glyph> {
            self.db();
            let db = self.db.as_ref()?;
            if !self.emojis.contains_key(&ch) {
                let resolved = crate::emoji_font::resolve(db, ch, pixels);
                self.emoji_bytes += resolved.as_ref().map_or(0, |r| r.bytes());
                self.emojis.insert(ch, resolved);
            }
            self.emoji_order.retain(|key| *key != ch);
            self.emoji_order.push_back(ch);
            // Font lookup is per character; decoded images live on their actual strike.
            let resolved = self.emojis.get_mut(&ch).and_then(Option::as_mut);
            let before = resolved.as_ref().map_or(0, |r| r.bytes());
            let (glyph, after) = resolved.map_or((None, 0), |r| (r.glyph(db, pixels), r.bytes()));
            self.emoji_bytes = self.emoji_bytes - before + after;
            while self.emojis.len() > 64 || self.emoji_bytes > crate::emoji_font::CACHED_BYTES {
                let Some(oldest) = self.emoji_order.pop_front() else {
                    break;
                };
                if let Some(Some(old)) = self.emojis.remove(&oldest) {
                    self.emoji_bytes -= old.bytes();
                }
            }
            glyph
        }

        fn db(&mut self) -> &Database {
            self.db.get_or_insert_with(|| {
                let db = Database::new();
                #[cfg(feature = "system-fonts")]
                let db = {
                    let mut db = db;
                    db.load_system_fonts();
                    self.installed = true;
                    db
                };
                db
            })
        }

        fn face_data(&mut self, id: ID) -> Option<(Arc<Vec<u8>>, u32)> {
            if let Some(cached) = self.faces.get(&id) {
                return cached.clone();
            }
            let loaded = self
                .db()
                .with_face_data(id, |data, index| (Arc::new(data.to_vec()), index));
            self.faces.insert(id, loaded.clone());
            loaded
        }

        fn query(&mut self, family: &str, weight: Weight, style: Style) -> Option<ID> {
            let families = [Family::Name(family)];
            self.db().query(&Query {
                families: &families,
                weight,
                stretch: Stretch::Normal,
                style,
            })
        }

        fn fallbacks(&mut self) -> Vec<ID> {
            if let Some(list) = &self.fallbacks {
                return list.clone();
            }
            self.db();
            // Without installed fonts, a name lookup would only find a supplied face out of
            // its supplied order (an emoji face ahead of the text face that sets line height).
            let mut list = if self.installed {
                Vec::new()
            } else {
                self.supplied.clone()
            };
            let mut found = Vec::new();
            for name in FALLBACK_FAMILIES.iter().chain(SYMBOL_FAMILIES) {
                found.extend(self.query(name, Weight::NORMAL, Style::Normal));
            }
            for generic in [Family::SansSerif, Family::Serif] {
                let families = [generic];
                found.extend(self.db().query(&Query {
                    families: &families,
                    ..Query::default()
                }));
            }
            found.extend(self.supplied.iter().copied());
            for id in found {
                if !list.contains(&id) {
                    list.push(id);
                }
            }
            self.fallbacks = Some(list.clone());
            list
        }

        /// The first face in `faces` that draws `ch`: its outline (font units), units per em
        /// and advance (em). Faces without outline tables (bitmap-only color emoji) are never
        /// used. A face whose glyph for `ch` is empty is passed over for a later one that
        /// outlines it; when none does, the first empty glyph is used, so a blank character
        /// (a space, U+2800 braille blank) keeps its advance instead of being dropped.
        pub(super) fn glyph_for(&mut self, faces: &[ID], ch: char) -> Option<(BezPath, f64, f64)> {
            let mut blank = None;
            for &id in faces {
                let Some((data, index)) = self.face_data(id) else {
                    continue;
                };
                let Ok(face) = Face::parse(&data, index) else {
                    continue;
                };
                let t = face.tables();
                if t.glyf.is_none() && t.cff.is_none() && t.cff2.is_none() {
                    continue;
                }
                let Some(g) = face.glyph_index(ch) else {
                    continue;
                };
                let upem = f64::from(face.units_per_em());
                let adv = face.glyph_hor_advance(g).map_or(0.0, f64::from) / upem;
                let mut builder = Builder(BezPath::new());
                if face.outline_glyph(g, &mut builder).is_some() {
                    return Some((builder.0, upem, adv));
                }
                // Whitespace never has an outline: the first face's blank is the answer.
                if ch.is_whitespace() {
                    return Some((BezPath::new(), upem, adv));
                }
                blank.get_or_insert((BezPath::new(), upem, adv));
            }
            blank
        }

        /// Lays out `textData.characters` with installed fonts. Returns outlines in node space,
        /// or `None` when no usable font was found.
        pub fn layout(&mut self, data: &Val) -> Option<Vec<BezPath>> {
            let text = super::characters(data);
            if text.trim().is_empty() {
                return Some(Vec::new());
            }
            let font_size = f64::from(data.num("fontSize").unwrap_or(12.0)).max(1.0);
            let family = data
                .path(&["fontName", "family"])
                .and_then(Val::as_str)
                .unwrap_or("Inter");
            let style_name = data
                .path(&["fontName", "style"])
                .and_then(Val::as_str)
                .unwrap_or("Regular");
            let style = if style_name.to_ascii_lowercase().contains("italic") {
                Style::Italic
            } else {
                Style::Normal
            };
            let mut faces = Vec::new();
            if let Some(id) = self.query(family, weight_of(style_name), style) {
                faces.push(id);
            }
            faces.extend(self.fallbacks());
            let primary = *faces.first()?;
            let (data_bytes, index) = self.face_data(primary)?;
            let (ascent, descent, natural) = {
                let face = Face::parse(&data_bytes, index).ok()?;
                let upem = f64::from(face.units_per_em());
                let asc = f64::from(face.ascender()) / upem * font_size;
                let desc = -f64::from(face.descender()) / upem * font_size;
                (
                    asc,
                    desc,
                    asc + desc + f64::from(face.line_gap()) / upem * font_size,
                )
            };

            let line_height = match (
                data.path(&["lineHeight", "units"]).and_then(Val::as_str),
                data.path(&["lineHeight", "value"]).and_then(Val::as_f32),
            ) {
                (Some("PIXELS"), Some(v)) if v > 0.0 => f64::from(v),
                (Some("PERCENT"), Some(v)) if v > 0.0 => f64::from(v) / 100.0 * font_size,
                (Some("RAW"), Some(v)) if v > 0.0 => f64::from(v) * font_size,
                _ => natural,
            };
            let spacing = match (
                data.path(&["letterSpacing", "units"]).and_then(Val::as_str),
                data.path(&["letterSpacing", "value"]).and_then(Val::as_f32),
            ) {
                (Some("PERCENT"), Some(v)) => f64::from(v) / 100.0 * font_size,
                (_, Some(v)) => f64::from(v),
                _ => 0.0,
            };
            let (w, h) = crate::geom::size(data);
            let wrap = data.text("textAutoResize") != Some("WIDTH_AND_HEIGHT") && w > 0.0;

            // Shape characters into lines (greedy wrap at spaces; CJK may break anywhere).
            let mut lines: Vec<Vec<Placed>> = vec![Vec::new()];
            let mut x = 0.0;
            let mut last_break: Option<usize> = None;
            for ch in text.chars() {
                if ch == '\n' || ch == '\u{2028}' {
                    lines.push(Vec::new());
                    x = 0.0;
                    last_break = None;
                    continue;
                }
                let Some((path, units_per_em, adv)) = self.glyph_for(&faces, ch) else {
                    continue;
                };
                let advance = adv * font_size + spacing;
                let line = lines.last_mut().unwrap();
                if wrap && x + advance > w + 0.5 && !line.is_empty() && !ch.is_whitespace() {
                    let cjk = ch > '\u{2e7f}';
                    let split = if cjk {
                        line.len()
                    } else {
                        last_break.map_or(line.len(), |b| b + 1)
                    };
                    let carried: Vec<Placed> = line.drain(split..).collect();
                    let shift = carried.first().map_or(0.0, |p| p.x);
                    let mut next: Vec<Placed> = carried
                        .into_iter()
                        .map(|mut p| {
                            p.x -= shift;
                            p
                        })
                        .collect();
                    x = next.last().map_or(0.0, |p| p.x + p.advance);
                    next.retain(|p| !(p.x == 0.0 && p.ch == ' '));
                    lines.push(next);
                    last_break = None;
                }
                let line = lines.last_mut().unwrap();
                if ch.is_whitespace() {
                    last_break = Some(line.len());
                }
                line.push(Placed {
                    path,
                    units_per_em,
                    x,
                    advance,
                    ch,
                });
                x += advance;
            }

            let align = data.text("textAlignHorizontal").unwrap_or("LEFT");
            let total = line_height * lines.len() as f64;
            let top = match data.text("textAlignVertical").unwrap_or("TOP") {
                "CENTER" => (h - total) / 2.0,
                "BOTTOM" => h - total,
                _ => 0.0,
            };
            let mut out = Vec::new();
            for (row, line) in lines.into_iter().enumerate() {
                let width = line
                    .iter()
                    .rev()
                    .find(|p| !p.ch.is_whitespace())
                    .map_or(0.0, |p| p.x + p.advance - spacing);
                let dx = match align {
                    "CENTER" => (w - width) / 2.0,
                    "RIGHT" => w - width,
                    _ => 0.0,
                };
                let baseline = top
                    + row as f64 * line_height
                    + (line_height - (ascent + descent)) / 2.0
                    + ascent;
                for placed in line {
                    if placed.path.elements().is_empty() {
                        continue;
                    }
                    let scale = font_size / placed.units_per_em;
                    let mut path = placed.path;
                    path.apply_affine(
                        Affine::translate((dx + placed.x, baseline))
                            * Affine::scale_non_uniform(scale, -scale),
                    );
                    out.push(path);
                }
            }
            Some(out)
        }
    }
}

#[cfg(all(test, feature = "system-fonts"))]
mod tests {
    use epaint_default_fonts::{EMOJI_ICON, HACK_REGULAR};

    use crate::text_test_font as test_font;
    use fontdb::{Database, ID};

    use super::SystemFonts;

    /// A font set of exactly `fonts` (no installed fonts), and their face ids in that order.
    fn fonts(fonts: &[&[u8]]) -> (SystemFonts, Vec<ID>) {
        let mut db = Database::new();
        let mut ids = Vec::new();
        for data in fonts {
            let before = db.len();
            db.load_font_data(data.to_vec());
            ids.extend(db.faces().skip(before).map(|f| f.id));
        }
        (SystemFonts::with_database(db), ids)
    }

    #[test]
    fn a_face_that_maps_the_character_without_an_outline_is_skipped() {
        // emoji-icon-font maps U+FFFF to a glyph with no outline and Hack outlines it: the
        // same shape as a bitmap-only color emoji face ahead of an outline face.
        let (mut f, ids) = fonts(&[EMOJI_ICON, HACK_REGULAR]);
        let (path, upem, advance) = f.glyph_for(&ids, '\u{FFFF}').unwrap();
        assert!(!path.elements().is_empty(), "drawn by the second face");
        assert_eq!(upem, 2048.0, "Hack's units per em");
        assert!(advance > 0.0);
    }

    #[test]
    fn a_blank_glyph_no_face_outlines_keeps_its_advance() {
        // Hack maps U+FEFF (not `char::is_whitespace`) to a glyph with an advance and no
        // contours, the shape of U+2800 braille blank: it must take its width, not vanish.
        let (mut f, ids) = fonts(&[HACK_REGULAR]);
        let (path, upem, advance) = f.glyph_for(&ids, '\u{FEFF}').unwrap();
        assert!(path.elements().is_empty());
        assert_eq!(advance * upem, 1233.0, "Hack's advance for the glyph");
    }

    #[test]
    fn whitespace_has_no_outline_but_still_advances() {
        let (mut f, ids) = fonts(&[HACK_REGULAR]);
        let (path, _, advance) = f.glyph_for(&ids, ' ').unwrap();
        assert!(path.elements().is_empty());
        assert!(advance > 0.0);
    }

    #[test]
    fn a_bitmap_only_face_does_not_set_the_space_width() {
        // A face without outline tables maps the space to a wide advance (2.55 em), like
        // Noto Color Emoji; the space must come from the outline face after it.
        let bitmap = test_font::bitmap_only(2550);
        let (mut f, ids) = fonts(&[&bitmap, HACK_REGULAR]);
        assert_eq!(ids.len(), 2, "both faces load");
        let (_, upem, advance) = f.glyph_for(&ids, ' ').unwrap();
        assert_eq!(upem, 2048.0, "Hack's space, not the bitmap face's");
        assert!(advance < 1.0, "{advance} em");
    }
}
