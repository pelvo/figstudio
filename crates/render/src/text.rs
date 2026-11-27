//! Text: stored glyph outlines and decoration rectangles first, installed fonts as a fallback.

use std::collections::HashMap;

use figstudio_fig::{Val, tokens::Tokens};
use vello_cpu::kurbo::{Affine, Rect, Shape};
use vello_cpu::peniko::Fill;

use crate::geom::GeomPath;

/// Styled glyph outlines, decoration bars, and approximation status in local text-node space.
#[derive(Debug, Default)]
pub(crate) struct TextPaths {
    pub glyphs: Vec<GeomPath>,
    pub decorations: Vec<GeomPath>,
    /// True when a requested decoration was missing from stored `derivedTextData.decorations`,
    /// used an unsupported `textDecorationStyle` (`DOTTED`/`WAVY`), or carried an unknown
    /// `textDecoration` variant.
    pub decoration_approximated: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextDecoration {
    None,
    Underline,
    Strikethrough,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Default)]
struct LineMetricsEm {
    /// Positive distance above baseline in em units (`OS/2.yStrikeoutPosition / upem`).
    strike_pos: Option<f64>,
    /// Positive strikeout thickness in em units (`OS/2.yStrikeoutSize / upem`).
    strike_thick: Option<f64>,
    /// Positive distance below baseline in em units (`-post.underlinePosition / upem`).
    underline_pos: Option<f64>,
    /// Positive underline thickness in em units (`post.underlineThickness / upem`).
    underline_thick: Option<f64>,
}

/// Fallback ratios used only when the requested OpenType face omits `OS/2` or `post` metrics.
const DEFAULT_THICKNESS_EM: f64 = 0.068;
const DEFAULT_STRIKE_ABOVE_EM: f64 = 0.33;
const DEFAULT_UNDERLINE_BELOW_EM: f64 = 0.21;

/// Reads a text-style field from `scope`, preferring an attached local `TEXT` style over
/// cached fields on `scope`.
fn scope_field<'a>(scope: &'a Val, tokens: &'a Tokens, key: &str) -> Option<&'a Val> {
    scope
        .get("styleIdForText")
        .or_else(|| scope.get("inheritTextStyleID"))
        .and_then(|r| tokens.style(r))
        .and_then(|s| s.data.get(key))
        .or_else(|| scope.get(key))
}

fn parse_decoration(value: Option<&Val>) -> Option<TextDecoration> {
    match value.and_then(Val::as_str) {
        Some("NONE") => Some(TextDecoration::None),
        Some("UNDERLINE") => Some(TextDecoration::Underline),
        Some("STRIKETHROUGH") => Some(TextDecoration::Strikethrough),
        Some(_) => Some(TextDecoration::Unsupported),
        None => None,
    }
}

fn decoration_from_scope(scope: &Val, tokens: &Tokens) -> Option<TextDecoration> {
    parse_decoration(scope_field(scope, tokens, "textDecoration"))
}

fn effective_decoration(
    data: &Val,
    overrides: &HashMap<u32, &Val>,
    style_id: u32,
    tokens: &Tokens,
) -> TextDecoration {
    if style_id != 0
        && let Some(ov) = overrides.get(&style_id).copied()
        && let Some(dec) = decoration_from_scope(ov, tokens)
    {
        return dec;
    }
    decoration_from_scope(data, tokens).unwrap_or(TextDecoration::None)
}

fn decoration_style_approximated(data: &Val, ov: Option<&Val>, tokens: &Tokens) -> bool {
    let field = ov
        .and_then(|o| scope_field(o, tokens, "textDecorationStyle"))
        .or_else(|| scope_field(data, tokens, "textDecorationStyle"));
    field.and_then(Val::as_str).is_some_and(|s| s != "SOLID")
}

fn style_override_map(data: &Val) -> HashMap<u32, &Val> {
    let table = data
        .path(&["textData", "styleOverrideTable"])
        .map(Val::items)
        .unwrap_or_default();
    let mut map = HashMap::with_capacity(table.len());
    for entry in table {
        if let Some(sid) = entry.get("styleID").and_then(Val::as_u32) {
            map.insert(sid, entry);
        }
    }
    map
}

fn style_override_entry(data: &Val, style_id: u32) -> Option<&Val> {
    if style_id == 0 {
        return None;
    }
    data.path(&["textData", "styleOverrideTable"])?
        .items()
        .iter()
        .find(|e| e.get("styleID").and_then(Val::as_u32) == Some(style_id))
}

/// Whether `data` (at node level, attached local `TEXT` style, or any referenced character run)
/// requests a non-`NONE` text decoration.
pub(crate) fn wants_decoration(data: &Val, tokens: &Tokens) -> bool {
    let node_dec = decoration_from_scope(data, tokens).unwrap_or(TextDecoration::None);
    let table = data
        .path(&["textData", "styleOverrideTable"])
        .map(Val::items)
        .unwrap_or_default();
    if table.is_empty() {
        return node_dec != TextDecoration::None;
    }
    let char_ids = data
        .path(&["textData", "characterStyleIDs"])
        .map(Val::items)
        .unwrap_or_default();
    if char_ids.is_empty() {
        return node_dec != TextDecoration::None;
    }
    let overrides = style_override_map(data);
    let text = characters(data);
    if text.is_empty() {
        return node_dec != TextDecoration::None;
    }
    let mut utf16_idx = 0usize;
    for ch in text.chars() {
        let style_id = char_ids.get(utf16_idx).and_then(Val::as_u32).unwrap_or(0);
        utf16_idx += ch.len_utf16();
        if effective_decoration(data, &overrides, style_id, tokens) != TextDecoration::None {
            return true;
        }
    }
    false
}

/// Maps Unicode scalar ordinals (`glyph.firstCharacter`) to UTF-16 code-unit indices
/// (`textData.characterStyleIDs`). Returns `None` for ASCII where the two indices are equal.
fn scalar_to_utf16_map(text: &str) -> Option<Vec<usize>> {
    if text.is_ascii() {
        return None;
    }
    let mut utf16 = 0usize;
    Some(
        text.chars()
            .map(|ch| {
                let at = utf16;
                utf16 += ch.len_utf16();
                at
            })
            .collect(),
    )
}

fn glyph_style_id(
    glyph: &Val,
    char_ids: &[Val],
    scalar_to_utf16: Option<&[usize]>,
    prev_style: u32,
) -> u32 {
    if let Some(sid) = glyph
        .get("styleID")
        .and_then(Val::as_u32)
        .filter(|&id| id != 0)
    {
        return sid;
    }
    let Some(scalar_idx) = glyph.get("firstCharacter").and_then(Val::as_u32) else {
        // A truncation ellipsis has no `firstCharacter` and inherits the preceding visible glyph.
        return prev_style;
    };
    let utf16_idx = match scalar_to_utf16 {
        Some(map) => match map.get(scalar_idx as usize).copied() {
            Some(i) => i,
            None => return 0,
        },
        None => scalar_idx as usize,
    };
    char_ids.get(utf16_idx).and_then(Val::as_u32).unwrap_or(0)
}

fn decoration_thickness(
    data: &Val,
    ov: Option<&Val>,
    tokens: &Tokens,
    font_size: f64,
    metric_em: Option<f64>,
) -> f64 {
    let field = ov
        .and_then(|o| scope_field(o, tokens, "textDecorationThickness"))
        .or_else(|| scope_field(data, tokens, "textDecorationThickness"));
    if let Some(v) = field {
        match (v.text("units"), v.num("value")) {
            (Some("PIXELS"), Some(n)) if n.is_finite() && n > 0.0 => return f64::from(n),
            (Some("PERCENT"), Some(n)) if n.is_finite() && n > 0.0 => {
                return f64::from(n) / 100.0 * font_size;
            }
            _ => {}
        }
    }
    let em = metric_em
        .filter(|&m| m.is_finite() && m > 0.0)
        .unwrap_or(DEFAULT_THICKNESS_EM);
    (em * font_size).max(0.5)
}

fn underline_offset(
    data: &Val,
    ov: Option<&Val>,
    tokens: &Tokens,
    font_size: f64,
    metric_em: Option<f64>,
) -> f64 {
    let field = ov
        .and_then(|o| scope_field(o, tokens, "textUnderlineOffset"))
        .or_else(|| scope_field(data, tokens, "textUnderlineOffset"));
    if let Some(v) = field {
        match (v.text("units"), v.num("value")) {
            (Some("PIXELS"), Some(n)) if n.is_finite() => return f64::from(n),
            (Some("PERCENT"), Some(n)) if n.is_finite() => {
                return f64::from(n) / 100.0 * font_size;
            }
            _ => {}
        }
    }
    let em = metric_em
        .filter(|&m| m.is_finite() && m > 0.0)
        .unwrap_or(DEFAULT_UNDERLINE_BELOW_EM);
    em * font_size
}

#[derive(Clone, Copy, Debug)]
struct DecorationSpan {
    x0: f64,
    x1: f64,
    baseline: f64,
    font_size: f64,
}

fn decoration_rect(
    dec: TextDecoration,
    data: &Val,
    ov: Option<&Val>,
    tokens: &Tokens,
    span: DecorationSpan,
    metrics: LineMetricsEm,
) -> Option<Rect> {
    if !(span.x0.is_finite()
        && span.x1.is_finite()
        && span.baseline.is_finite()
        && span.font_size.is_finite())
        || span.x1 <= span.x0
        || span.font_size <= 0.0
    {
        return None;
    }
    let (y_top, h) = match dec {
        TextDecoration::None | TextDecoration::Unsupported => return None,
        TextDecoration::Strikethrough => {
            let h = decoration_thickness(data, ov, tokens, span.font_size, metrics.strike_thick);
            let above = metrics
                .strike_pos
                .filter(|&m| m.is_finite() && m > 0.0)
                .unwrap_or(DEFAULT_STRIKE_ABOVE_EM)
                * span.font_size;
            (span.baseline - above, h)
        }
        TextDecoration::Underline => {
            let h = decoration_thickness(data, ov, tokens, span.font_size, metrics.underline_thick);
            let below = underline_offset(data, ov, tokens, span.font_size, metrics.underline_pos);
            (span.baseline + below, h)
        }
    };
    (y_top.is_finite() && h.is_finite() && h > 0.0).then_some(Rect::new(
        span.x0,
        y_top,
        span.x1,
        y_top + h,
    ))
}

/// Stored glyph outlines (`derivedTextData.glyphs`) and authoritative stored decorations
/// (`derivedTextData.decorations`) in node space. Does not synthesize heuristic bars when
/// usable stored decorations are absent; instead flags `decoration_approximated` if a non-`NONE`
/// decoration was requested without usable stored rectangles.
pub(crate) fn stored(data: &Val, blobs: &[Vec<u8>], tokens: &Tokens) -> TextPaths {
    let glyphs = visible_glyphs(data);
    if glyphs.is_empty() {
        return TextPaths::default();
    }
    let stored_decs = data
        .path(&["derivedTextData", "decorations"])
        .map(Val::items)
        .unwrap_or_default();
    // Only map per-glyph run style IDs when stored decorations are present and a text style
    // override actually replaces fillPaints; undecorated text keeps zero per-glyph overhead.
    let needs_glyph_styles = !stored_decs.is_empty()
        && data
            .path(&["textData", "styleOverrideTable"])
            .is_some_and(|t| t.items().iter().any(|e| e.get("fillPaints").is_some()));
    let char_ids = if needs_glyph_styles {
        data.path(&["textData", "characterStyleIDs"])
            .map(Val::items)
            .unwrap_or_default()
    } else {
        &[]
    };
    let scalar_to_utf16 = if needs_glyph_styles && !char_ids.is_empty() {
        scalar_to_utf16_map(characters(data))
    } else {
        None
    };

    let mut out = TextPaths::default();
    out.glyphs.reserve(glyphs.len());
    let mut has_emoji = false;
    let mut prev_style = 0u32;

    for glyph in glyphs {
        let style = if needs_glyph_styles {
            let sid = glyph_style_id(glyph, char_ids, scalar_to_utf16.as_deref(), prev_style);
            prev_style = sid;
            sid
        } else {
            0
        };
        if is_emoji(glyph) {
            has_emoji = true;
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
        out.glyphs.push(GeomPath {
            path,
            rule: Fill::NonZero,
            style,
        });
    }

    if out.glyphs.is_empty() && !has_emoji {
        return out;
    }

    if !stored_decs.is_empty() {
        for dec in stored_decs {
            let style = dec.get("styleID").and_then(Val::as_u32).unwrap_or(0);
            let ov = style_override_entry(data, style);
            if decoration_style_approximated(data, ov, tokens) {
                out.decoration_approximated = true;
            }
            for r in dec.get("rects").map(Val::items).unwrap_or_default() {
                let (Some(x), Some(y), Some(w), Some(h)) =
                    (r.num("x"), r.num("y"), r.num("w"), r.num("h"))
                else {
                    continue;
                };
                let (x, y, w, h) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
                if x.is_finite()
                    && y.is_finite()
                    && w.is_finite()
                    && h.is_finite()
                    && w > 0.0
                    && h > 0.0
                {
                    out.decorations.push(GeomPath {
                        path: Rect::new(x, y, x + w, y + h).to_path(0.1),
                        rule: Fill::NonZero,
                        style,
                    });
                }
            }
        }
    }
    if out.decorations.is_empty() && wants_decoration(data, tokens) {
        out.decoration_approximated = true;
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

    use figstudio_fig::{Val, tokens::Tokens};
    use fontdb::{Database, Family, ID, Query, Stretch, Style, Weight};
    use ttf_parser::{Face, OutlineBuilder};
    use vello_cpu::kurbo::{Affine, BezPath, Shape};
    use vello_cpu::peniko::Fill;

    use super::{
        DecorationSpan, LineMetricsEm, TextDecoration, TextPaths, decoration_from_scope,
        decoration_rect, decoration_style_approximated, effective_decoration, style_override_map,
    };
    use crate::geom::GeomPath;

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
        font_size: f64,
        x: f64,
        advance: f64,
        spacing: f64,
        ch: char,
        style_id: u32,
        decoration: TextDecoration,
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

        fn face_line_metrics(face: &Face<'_>, upem: f64) -> LineMetricsEm {
            let strike = face.strikeout_metrics();
            let ul = face.underline_metrics();
            LineMetricsEm {
                strike_pos: strike
                    .filter(|m| m.position > 0)
                    .map(|m| f64::from(m.position) / upem),
                strike_thick: strike
                    .filter(|m| m.thickness > 0)
                    .map(|m| f64::from(m.thickness) / upem),
                underline_pos: ul
                    .filter(|m| m.position < 0)
                    .map(|m| -f64::from(m.position) / upem),
                underline_thick: ul
                    .filter(|m| m.thickness > 0)
                    .map(|m| f64::from(m.thickness) / upem),
            }
        }

        fn font_name_line_metrics(
            &mut self,
            fn_val: &Val,
            default_family: &str,
            default_style: &str,
        ) -> Option<LineMetricsEm> {
            let fam = fn_val.text("family").unwrap_or(default_family);
            let sty_name = fn_val.text("style").unwrap_or(default_style);
            let sty = if sty_name.to_ascii_lowercase().contains("italic") {
                Style::Italic
            } else {
                Style::Normal
            };
            let id = self.query(fam, weight_of(sty_name), sty)?;
            let (data_bytes, index) = self.face_data(id)?;
            let face = Face::parse(&data_bytes, index).ok()?;
            let upem = f64::from(face.units_per_em());
            Some(Self::face_line_metrics(&face, upem))
        }

        /// Lays out `textData.characters` with installed fonts. Returns styled glyph outlines and
        /// decoration bars in node space, or `None` when no usable font was found.
        pub(crate) fn layout(&mut self, data: &Val, tokens: &Tokens) -> Option<TextPaths> {
            let text = super::characters(data);
            if text.trim().is_empty() {
                return Some(TextPaths::default());
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

            // Optional per-UTF-16-unit mixed style overrides (`characterStyleIDs` + `styleOverrideTable`).
            let char_style_ids = data
                .path(&["textData", "characterStyleIDs"])
                .map(Val::items)
                .unwrap_or_default();
            let override_map = style_override_map(data);
            let node_dec = decoration_from_scope(data, tokens).unwrap_or(TextDecoration::None);
            let has_decorations = node_dec != TextDecoration::None
                || override_map.values().any(|ov| {
                    decoration_from_scope(ov, tokens).is_some_and(|d| d != TextDecoration::None)
                });

            let (data_bytes, index) = self.face_data(primary)?;
            let (ascent, descent, natural, primary_metrics) = {
                let face = Face::parse(&data_bytes, index).ok()?;
                let upem = f64::from(face.units_per_em());
                let asc = f64::from(face.ascender()) / upem * font_size;
                let desc = -f64::from(face.descender()) / upem * font_size;
                let metrics = if has_decorations {
                    Self::face_line_metrics(&face, upem)
                } else {
                    LineMetricsEm::default()
                };
                (
                    asc,
                    desc,
                    asc + desc + f64::from(face.line_gap()) / upem * font_size,
                    metrics,
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
            let mut utf16_idx = 0usize;
            let mut decoration_approximated = false;
            for ch in text.chars() {
                let ch_u16 = ch.len_utf16();
                let style_id = char_style_ids
                    .get(utf16_idx)
                    .and_then(Val::as_u32)
                    .unwrap_or(0);
                utf16_idx += ch_u16;

                if ch == '\n' || ch == '\u{2028}' {
                    lines.push(Vec::new());
                    x = 0.0;
                    last_break = None;
                    continue;
                }
                let ov = (style_id != 0)
                    .then(|| override_map.get(&style_id).copied())
                    .flatten();
                let decoration = if has_decorations {
                    let dec = effective_decoration(data, &override_map, style_id, tokens);
                    if dec == TextDecoration::Unsupported {
                        decoration_approximated = true;
                    }
                    dec
                } else {
                    TextDecoration::None
                };
                let ch_font_size = ov
                    .and_then(|o| o.num("fontSize"))
                    .map_or(font_size, |s| f64::from(s).max(1.0));
                let ch_spacing = match ov.and_then(|o| o.get("letterSpacing")) {
                    Some(ls) => match (ls.text("units"), ls.num("value")) {
                        (Some("PERCENT"), Some(v)) => f64::from(v) / 100.0 * ch_font_size,
                        (_, Some(v)) => f64::from(v),
                        _ => spacing,
                    },
                    None => spacing,
                };
                let custom_faces = ov.and_then(|o| o.get("fontName")).map(|fn_val| {
                    let fam = fn_val.text("family").unwrap_or(family);
                    let sty_name = fn_val.text("style").unwrap_or(style_name);
                    let sty = if sty_name.to_ascii_lowercase().contains("italic") {
                        Style::Italic
                    } else {
                        Style::Normal
                    };
                    let mut list = Vec::new();
                    if let Some(id) = self.query(fam, weight_of(sty_name), sty) {
                        list.push(id);
                    }
                    list.extend(self.fallbacks());
                    list
                });
                let use_faces = custom_faces.as_deref().unwrap_or(&faces);
                let Some((path, units_per_em, adv)) = self.glyph_for(use_faces, ch) else {
                    continue;
                };
                let advance = adv * ch_font_size + ch_spacing;
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
                    font_size: ch_font_size,
                    x,
                    advance,
                    spacing: ch_spacing,
                    ch,
                    style_id,
                    decoration,
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
            let mut out = TextPaths {
                decoration_approximated,
                ..TextPaths::default()
            };
            for (row, line) in lines.into_iter().enumerate() {
                let last_non_ws = line.iter().rposition(|p| !p.ch.is_whitespace());
                let width = last_non_ws.map_or(0.0, |idx| {
                    let p = &line[idx];
                    p.x + p.advance - p.spacing
                });
                let dx = match align {
                    "CENTER" => (w - width) / 2.0,
                    "RIGHT" => w - width,
                    _ => 0.0,
                };
                let baseline = top
                    + row as f64 * line_height
                    + (line_height - (ascent + descent)) / 2.0
                    + ascent;
                if has_decorations && let Some(end_idx) = last_non_ws {
                    let mut i = 0;
                    while i <= end_idx {
                        let p = &line[i];
                        if !matches!(
                            p.decoration,
                            TextDecoration::Underline | TextDecoration::Strikethrough
                        ) {
                            i += 1;
                            continue;
                        }
                        let mut j = i + 1;
                        while j <= end_idx
                            && line[j].decoration == p.decoration
                            && line[j].style_id == p.style_id
                            && (line[j].font_size - p.font_size).abs() <= 0.05
                        {
                            j += 1;
                        }
                        let last = &line[j - 1];
                        let next_decorated = j <= end_idx
                            && matches!(
                                line[j].decoration,
                                TextDecoration::Underline | TextDecoration::Strikethrough
                            );
                        let x0 = dx + p.x;
                        let x1 = dx + last.x + last.advance
                            - if next_decorated { 0.0 } else { last.spacing };
                        let ov = (p.style_id != 0)
                            .then(|| override_map.get(&p.style_id).copied())
                            .flatten();
                        if decoration_style_approximated(data, ov, tokens) {
                            out.decoration_approximated = true;
                        }
                        let metrics = ov
                            .and_then(|o| o.get("fontName"))
                            .and_then(|fn_val| {
                                self.font_name_line_metrics(fn_val, family, style_name)
                            })
                            .unwrap_or(primary_metrics);
                        if let Some(rect) = decoration_rect(
                            p.decoration,
                            data,
                            ov,
                            tokens,
                            DecorationSpan {
                                x0,
                                x1,
                                baseline,
                                font_size: p.font_size,
                            },
                            metrics,
                        ) {
                            out.decorations.push(GeomPath {
                                path: rect.to_path(0.1),
                                rule: Fill::NonZero,
                                style: p.style_id,
                            });
                        }
                        i = j;
                    }
                }
                for placed in line {
                    if placed.path.elements().is_empty() {
                        continue;
                    }
                    let scale = placed.font_size / placed.units_per_em;
                    let mut path = placed.path;
                    path.apply_affine(
                        Affine::translate((dx + placed.x, baseline))
                            * Affine::scale_non_uniform(scale, -scale),
                    );
                    out.glyphs.push(GeomPath {
                        path,
                        rule: Fill::NonZero,
                        style: if has_decorations { placed.style_id } else { 0 },
                    });
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
