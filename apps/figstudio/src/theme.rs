//! Arc-style look: soft per-file gradients behind floating cards, in light and dark.
//!
//! The window background is a gradient tinted per open file ([`Tint`], like Arc spaces). The
//! sidebar sits straight on it with translucent rows; the canvas and the properties panel are
//! opaque rounded cards, the properties card washed in the space's colour
//! ([`Palette::on_tint`]). [`Palette`] holds the colours of one appearance; the app follows the
//! system setting unless View > Appearance overrides it.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Theme, Visuals};

/// Selection and hover outlines on the canvas, and the marquee: the same blue in both themes.
pub const ACCENT: Color32 = Color32::from_rgb(55, 142, 240);

/// Name of the semibold UI face (titles, the active page); falls back to the regular face.
pub const SEMIBOLD: &str = "semibold";

/// Corner radius of the canvas and properties cards.
pub const CARD_RADIUS: u8 = 12;
/// Gap between the window edge, the sidebar and the cards.
pub const GAP: f32 = 8.0;
/// Opacity of the window gradient over the macOS blur: tinted, with the desktop showing
/// through.
pub const TRANSLUCENT_TINT: f32 = 0.6;

/// White at `a` alpha (premultiplied), for translucent fills on the gradient.
const fn white(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}

/// The colours of one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    /// Row and button hover on the window gradient.
    pub hover: Color32,
    /// The selected row (active page, selected layer) on the window gradient.
    pub selected: Color32,
    /// Fields on the window gradient (the command field, the layer filter).
    pub field: Color32,
    /// The canvas and properties cards.
    pub card: Color32,
    pub card_stroke: Color32,
    /// Grouped rows inside a card (properties sections).
    pub well: Color32,
    /// Floating pills over the canvas (tools, status, toasts) and the command bar.
    pub glass: Color32,
    pub separator: Color32,
    pub shadow: Color32,
    /// Components and instances in the layer tree.
    pub component: Color32,
    pub warning: Color32,
}

pub const LIGHT: Palette = Palette {
    dark: false,
    text: Color32::from_rgb(28, 28, 32),
    text_dim: Color32::from_rgb(84, 84, 94),
    text_faint: Color32::from_rgb(112, 112, 122),
    hover: white(80),
    selected: white(210),
    field: white(120),
    card: Color32::WHITE,
    card_stroke: Color32::from_black_alpha(20),
    well: Color32::from_rgb(244, 244, 247),
    glass: Color32::from_rgba_premultiplied(232, 232, 232, 232),
    separator: Color32::from_black_alpha(22),
    shadow: Color32::from_black_alpha(34),
    component: Color32::from_rgb(124, 77, 255),
    warning: Color32::from_rgb(176, 104, 0),
};

pub const DARK: Palette = Palette {
    dark: true,
    text: Color32::from_rgb(236, 236, 241),
    text_dim: Color32::from_rgb(170, 170, 180),
    text_faint: Color32::from_rgb(140, 140, 152),
    hover: white(16),
    selected: white(36),
    field: white(22),
    card: Color32::from_rgb(28, 28, 31),
    card_stroke: white(20),
    well: Color32::from_rgb(36, 36, 40),
    glass: Color32::from_rgba_premultiplied(38, 38, 42, 236),
    separator: white(22),
    shadow: Color32::from_black_alpha(120),
    component: Color32::from_rgb(178, 150, 255),
    warning: Color32::from_rgb(240, 180, 70),
};

/// How much of a space's colour the properties card and its wells take.
const CARD_TINT: f32 = 0.5;

impl Palette {
    /// This palette with the card and its wells washed in the space's `tint`, so the
    /// properties card follows the space's colour. Text stays readable: the tint's own
    /// colours already are, and these lie between them and the neutral card.
    pub fn on_tint(self, tint: Tint) -> Palette {
        let card = self.card.lerp_to_gamma(tint.top, CARD_TINT);
        // The well keeps its neutral step from the card. Mixing both into the same colour
        // would halve that step and flatten the wells into the card.
        let step = |card: u8, neutral_card: u8, neutral_well: u8| {
            let v = i16::from(card) + i16::from(neutral_well) - i16::from(neutral_card);
            u8::try_from(v.clamp(0, 255)).unwrap_or(u8::MAX)
        };
        let well = Color32::from_rgb(
            step(card.r(), self.card.r(), self.well.r()),
            step(card.g(), self.card.g(), self.well.g()),
            step(card.b(), self.card.b(), self.well.b()),
        );
        Palette { card, well, ..self }
    }
}

/// The palette of the context's current appearance.
pub fn palette(ctx: &egui::Context) -> Palette {
    match ctx.theme() {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

/// The palette of `ui`'s appearance.
pub fn of(ui: &egui::Ui) -> Palette {
    if ui.visuals().dark_mode { DARK } else { LIGHT }
}

/// A file's window gradient: `top` at the top left, `bottom` at the bottom right, a soft
/// `glow` near the top, and an `accent` for the chrome highlights (the active tool, the file
/// switcher).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tint {
    pub top: Color32,
    pub bottom: Color32,
    pub glow: Color32,
    pub accent: Color32,
}

impl Tint {
    /// Between `self` (at 0) and `other` (at 1).
    pub fn mix(self, other: Tint, t: f32) -> Tint {
        Tint {
            top: self.top.lerp_to_gamma(other.top, t),
            bottom: self.bottom.lerp_to_gamma(other.bottom, t),
            glow: self.glow.lerp_to_gamma(other.glow, t),
            accent: self.accent.lerp_to_gamma(other.accent, t),
        }
    }
}

/// How many tints there are; [`pick_tint`] hands each open file a different one while it can.
pub const TINTS: usize = 8;

fn tint_of(c: [[u8; 3]; 4]) -> Tint {
    let rgb = |[r, g, b]: [u8; 3]| Color32::from_rgb(r, g, b);
    Tint {
        top: rgb(c[0]),
        bottom: rgb(c[1]),
        glow: rgb(c[2]),
        accent: rgb(c[3]),
    }
}

/// Lavender, sky, mint, peach, rose, lime, sand, slate.
const LIGHT_TINTS: [[[u8; 3]; 4]; TINTS] = [
    [
        [230, 222, 255],
        [204, 212, 252],
        [252, 226, 246],
        [112, 82, 232],
    ],
    [
        [216, 234, 255],
        [196, 220, 250],
        [230, 248, 255],
        [30, 116, 228],
    ],
    [
        [214, 245, 233],
        [192, 230, 222],
        [238, 252, 222],
        [12, 140, 128],
    ],
    [
        [255, 233, 216],
        [250, 212, 202],
        [255, 245, 218],
        [222, 100, 32],
    ],
    [
        [255, 224, 236],
        [244, 206, 226],
        [255, 238, 224],
        [210, 36, 112],
    ],
    [
        [234, 246, 210],
        [212, 236, 204],
        [250, 252, 216],
        [72, 132, 18],
    ],
    [
        [245, 237, 224],
        [230, 221, 206],
        [253, 247, 234],
        [150, 102, 36],
    ],
    [
        [229, 233, 241],
        [212, 218, 230],
        [242, 245, 252],
        [76, 92, 126],
    ],
];

const DARK_TINTS: [[[u8; 3]; 4]; TINTS] = [
    [[46, 38, 86], [22, 20, 44], [68, 42, 104], [164, 136, 255]],
    [[22, 48, 88], [13, 23, 46], [22, 62, 98], [100, 168, 250]],
    [[16, 62, 58], [11, 30, 32], [16, 68, 60], [52, 212, 190]],
    [[84, 44, 24], [38, 21, 16], [94, 50, 24], [251, 150, 66]],
    [[80, 28, 52], [36, 16, 27], [96, 34, 62], [244, 118, 184]],
    [[40, 66, 24], [19, 31, 14], [44, 68, 22], [164, 228, 60]],
    [[70, 56, 36], [33, 27, 20], [74, 58, 36], [232, 180, 20]],
    [[42, 46, 58], [21, 23, 29], [50, 55, 70], [150, 166, 188]],
];

/// Tint number `index` (wrapped) for the given appearance.
pub fn tint(index: usize, dark: bool) -> Tint {
    let table = if dark { &DARK_TINTS } else { &LIGHT_TINTS };
    tint_of(table[index % TINTS])
}

/// Names of the preset tints, in order (the space editor's swatches).
pub const TINT_NAMES: [&str; TINTS] = [
    "Lavender", "Sky", "Mint", "Peach", "Rose", "Lime", "Sand", "Slate",
];

/// A space's colour: one of the preset tints, or any colour the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Colour {
    Preset(usize),
    Custom([u8; 3]),
}

/// The window gradient for a space's `colour`.
pub fn space_tint(colour: Colour, dark: bool) -> Tint {
    match colour {
        Colour::Preset(i) => tint(i, dark),
        Colour::Custom([r, g, b]) => custom_tint(Color32::from_rgb(r, g, b), dark),
    }
}

/// WCAG contrast ratio of two opaque sRGB colours (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    fn luminance(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// Black or white, whichever reads better on `fill` (text on an accent-coloured button).
pub fn on_accent(fill: Color32) -> Color32 {
    let dark_text = Color32::from_rgb(20, 20, 24);
    if contrast(Color32::WHITE, fill) >= contrast(dark_text, fill) {
        Color32::WHITE
    } else {
        dark_text
    }
}

/// A gradient made from any colour: as much of `c` as keeps the palette's text readable on
/// it, and an accent that stands out from the cards.
fn custom_tint(c: Color32, dark: bool) -> Tint {
    let p = if dark { DARK } else { LIGHT };
    let readable = |bg: Color32| {
        contrast(p.text, bg) >= 7.0
            && contrast(p.text_dim, bg) >= 4.5
            && contrast(p.text_faint, bg) >= 3.0
    };
    // The strongest mix of `c` into `base`, up to `most`, with readable text on it.
    let mix = |base: Color32, most: f32| {
        (0..=20)
            .rev()
            .map(|i| base.lerp_to_gamma(c, most * i as f32 / 20.0))
            .find(|bg| readable(*bg))
            .unwrap_or(base)
    };
    let (top, bottom, glow) = if dark {
        let base = Color32::from_rgb(22, 22, 26);
        (
            mix(base, 0.4),
            mix(Color32::from_rgb(12, 12, 15), 0.22),
            mix(base, 0.6),
        )
    } else {
        let base = Color32::from_rgb(250, 250, 252);
        (mix(base, 0.2), mix(base, 0.34), mix(Color32::WHITE, 0.1))
    };
    // The accent marks the active tool and file: it must stand out from the cards.
    let toward = if dark { Color32::WHITE } else { Color32::BLACK };
    // And button text on it (`on_accent`) must read at 4.5:1, which mid tones miss.
    let accent = (0..=20)
        .map(|i| c.lerp_to_gamma(toward, i as f32 / 20.0))
        .find(|a| contrast(*a, p.card) >= 3.0 && contrast(on_accent(*a), *a) >= 4.5)
        .unwrap_or(toward);
    Tint {
        top,
        bottom,
        glow,
        accent,
    }
}

/// The tint shown when no file is open.
pub const HOME_TINT: usize = 0;

/// A tint for a file named `name`: the same file gets the same colour each time, unless an
/// open file (`taken`) already has it, then the next free one.
pub fn pick_tint(name: &str, taken: impl IntoIterator<Item = usize>) -> usize {
    // FNV-1a: a stable spread of names over the tints (std's hasher may change between builds).
    let hash = name.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    let start = hash as usize % TINTS;
    let taken: Vec<usize> = taken.into_iter().collect();
    (0..TINTS)
        .map(|i| (start + i) % TINTS)
        .find(|i| !taken.contains(i))
        .unwrap_or(start)
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.override_text_color = Some(p.text);
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = if p.dark {
        Color32::from_rgb(34, 34, 38)
    } else {
        Color32::WHITE
    };
    v.window_stroke = Stroke::new(1.0, p.card_stroke);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = Shadow {
        offset: [0, 10],
        blur: 32,
        spread: 0,
        color: p.shadow,
    };
    v.popup_shadow = Shadow {
        offset: [0, 6],
        blur: 20,
        spread: 0,
        color: p.shadow,
    };
    v.extreme_bg_color = p.well;
    v.faint_bg_color = p.well;
    v.code_bg_color = p.well;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = p.warning;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    let fill = |a: u8| {
        if p.dark {
            white(a)
        } else {
            Color32::from_black_alpha(a)
        }
    };
    let r = CornerRadius::same(7);
    for (w, a) in [
        (&mut v.widgets.noninteractive, 0),
        (&mut v.widgets.inactive, 14),
        (&mut v.widgets.hovered, 24),
        (&mut v.widgets.active, 34),
        (&mut v.widgets.open, 24),
    ] {
        w.corner_radius = r;
        w.expansion = 0.0;
        if a > 0 {
            w.weak_bg_fill = fill(a);
            w.bg_fill = fill(a);
            w.bg_stroke = Stroke::NONE;
            w.fg_stroke = Stroke::new(1.0, p.text);
        }
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);
    v
}

/// The highlight of the row under the pointer in menus, as macOS draws it.
pub const MENU_HIGHLIGHT: Color32 = Color32::from_rgb(10, 105, 230);

/// Menus in the macOS style (Arc uses the system's): rounded, roomy rows, a blue highlight
/// with white text. Used for every popup and context menu the app opens.
pub fn menu_style(style: &mut egui::Style) {
    egui::containers::menu::menu_style(style);
    let dark = style.visuals.dark_mode;
    let p = if dark { DARK } else { LIGHT };
    let v = &mut style.visuals;
    v.override_text_color = None;
    v.window_fill = if dark {
        Color32::from_rgb(40, 40, 44)
    } else {
        Color32::from_rgb(246, 246, 248)
    };
    v.window_stroke = Stroke::new(
        1.0,
        if dark {
            white(34)
        } else {
            Color32::from_black_alpha(30)
        },
    );
    v.menu_corner_radius = CornerRadius::same(10);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    for w in [
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.weak_bg_fill = MENU_HIGHLIGHT;
        w.bg_fill = MENU_HIGHLIGHT;
        w.bg_stroke = Stroke::NONE;
        w.fg_stroke = Stroke::new(1.0, Color32::WHITE);
        w.corner_radius = CornerRadius::same(5);
        w.expansion = 0.0;
    }
    style.spacing.button_padding = egui::vec2(10.0, 3.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.menu_margin = Margin::same(5);
    style.spacing.item_spacing = egui::vec2(6.0, 1.0);
}

/// Installs both appearances; egui picks one from the system setting (or View > Appearance).
pub fn apply(ctx: &egui::Context) {
    // A host that turned egui's animations off (the test harness) keeps them off.
    let still = crate::anim::reduced(ctx);
    ctx.set_visuals_of(Theme::Dark, visuals(&DARK));
    ctx.set_visuals_of(Theme::Light, visuals(&LIGHT));
    ctx.all_styles_mut(|s| {
        use egui::TextStyle;
        s.spacing.item_spacing = egui::vec2(6.0, 5.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(14);
        s.animation_time = if still {
            0.0
        } else {
            crate::anim::EGUI_ANIMATION_TIME
        };
        for (style, size) in [
            (TextStyle::Body, 13.0),
            (TextStyle::Button, 13.0),
            (TextStyle::Small, 11.0),
            (TextStyle::Heading, 20.0),
            (TextStyle::Monospace, 12.0),
        ] {
            if let Some(font) = s.text_styles.get_mut(&style) {
                font.size = size;
            }
        }
    });
}

/// The semibold UI face at `size` points.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Makes sure the [`SEMIBOLD`] family exists (with the regular faces as its fallback).
fn add_semibold_family(fonts: &mut egui::FontDefinitions, face: Option<&str>) {
    let regular = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = face.map(str::to_owned).into_iter().chain(regular).collect();
    fonts
        .families
        .insert(FontFamily::Name(SEMIBOLD.into()), names);
}

/// Browser build: egui's own faces (no installed fonts to read) plus the [`SEMIBOLD`] alias.
/// A CJK font is fetched when a file needs one (`web::Inbox::load_cjk_font`, after
/// [`cjk_need`]).
#[cfg(target_arch = "wasm32")]
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    add_semibold_family(&mut fonts, None);
    ctx.set_fonts(fonts);
}

/// Hangul syllables and jamo: only the KR face of the browser's CJK fallback draws them.
fn is_hangul(c: char) -> bool {
    matches!(c, '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{AC00}'..='\u{D7AF}')
}

/// Whether `c` needs a CJK font (egui's built-in fonts cover Latin, Greek and Cyrillic).
pub fn is_cjk(c: char) -> bool {
    is_hangul(c)
        || matches!(c,
            '\u{2E80}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FE30}'..='\u{FE4F}'
            | '\u{FF00}'..='\u{FFEF}')
}

/// CJK punctuation and full-width forms: every face of the browser's fallback has them, so
/// they need a CJK font but not a particular one.
fn is_cjk_shared(c: char) -> bool {
    matches!(c, '\u{3000}'..='\u{303F}' | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}')
}

/// What the CJK fallback has to draw for a document.
/// Ordered: a document needs the largest of what its texts need.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CjkNeed {
    None,
    /// Only punctuation and full-width forms, which every face has.
    Shared,
    /// Hangul and no kana or ideographs: only the KR face draws it.
    HangulOnly,
    /// Kana, bopomofo or ideographs: the chosen face.
    Any,
}

impl CjkNeed {
    fn add(self, text: &str) -> Self {
        text.chars().fold(self, |need, c| {
            need.max(if is_hangul(c) {
                CjkNeed::HangulOnly
            } else if is_cjk_shared(c) {
                CjkNeed::Shared
            } else if is_cjk(c) {
                CjkNeed::Any
            } else {
                CjkNeed::None
            })
        })
    }
}

/// What CJK anything the UI shows for this document (file name, page and layer names, text
/// contents) needs.
pub fn cjk_need(doc: &figstudio_fig::Document, file_name: &str) -> CjkNeed {
    let mut need = CjkNeed::None.add(file_name);
    if let Some(name) = doc.file_name.as_deref() {
        need = need.add(name);
    }
    for n in &doc.nodes {
        if need == CjkNeed::Any {
            break;
        }
        need = need.add(&n.name);
        if let Some(text) = n
            .data
            .path(&["textData", "characters"])
            .and_then(figstudio_fig::Val::as_str)
        {
            need = need.add(text);
        }
    }
    need
}

/// Adds `bytes` (a CJK font) as the last fallback of every family.
#[cfg_attr(
    not(any(test, target_arch = "wasm32")),
    expect(dead_code, reason = "only the browser fetches a CJK font")
)]
pub fn add_cjk_font(ctx: &egui::Context, bytes: Vec<u8>) {
    // add_font ignores an existing name; set_fonts replaces it and preserves other fonts.
    let mut fonts = ctx.fonts(|fonts| fonts.definitions().clone());
    fonts
        .font_data
        .insert("cjk".into(), egui::FontData::from_owned(bytes).into());
    for family in [
        FontFamily::Proportional,
        FontFamily::Monospace,
        FontFamily::Name(SEMIBOLD.into()),
    ] {
        let names = fonts.families.entry(family).or_default();
        names.retain(|name| name != "cjk");
        names.push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

/// One installed face, read once per process.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
struct Face {
    bytes: Vec<u8>,
    index: u32,
}

#[cfg(not(target_arch = "wasm32"))]
impl Face {
    fn data(&'static self, weight: Option<f32>) -> egui::FontData {
        let mut tweak = egui::epaint::text::FontTweak::default();
        if let Some(weight) = weight {
            tweak.coords = egui::epaint::text::VariationCoords::new([(*b"wght", weight)]);
        }
        egui::FontData {
            font: std::borrow::Cow::Borrowed(&self.bytes),
            index: self.index,
            tweak,
        }
    }
}

/// The installed fonts the UI uses, looked up once (a system font scan takes a while).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default)]
struct SystemFonts {
    /// The platform's UI face (San Francisco, Segoe UI, Inter, …).
    ui: Option<Face>,
    /// Its semibold face; `None` when `ui` is variable (then a `wght` coordinate makes it).
    ui_semibold: Option<Face>,
    ui_variable: bool,
    cjk: Option<Face>,
    /// Faces with the symbols the UI face lacks (⌞, ⚠, …), with real outlines. Several:
    /// no single family covers them all.
    symbols: Vec<Face>,
}

#[cfg(not(target_arch = "wasm32"))]
fn system_fonts() -> &'static SystemFonts {
    static FONTS: std::sync::OnceLock<SystemFonts> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let find = |name: &str, weight: fontdb::Weight| {
            let families = [fontdb::Family::Name(name)];
            let id = db.query(&fontdb::Query {
                families: &families,
                weight,
                ..Default::default()
            })?;
            let face = db.face(id)?;
            if face.weight != weight {
                return None;
            }
            db.with_face_data(id, |data, index| Face {
                bytes: data.to_vec(),
                index,
            })
        };
        // UI faces in order of preference; the first one installed wins.
        const UI: [&str; 9] = [
            "System Font",
            "SF Pro Text",
            "Segoe UI Variable Text",
            "Segoe UI",
            "Inter Variable",
            "Inter",
            "Cantarell",
            "Noto Sans",
            "Ubuntu",
        ];
        let mut fonts = SystemFonts::default();
        for name in UI {
            let Some(face) = find(name, fontdb::Weight::NORMAL) else {
                continue;
            };
            let mut data = egui::FontData::from_owned(face.bytes.clone());
            data.index = face.index;
            fonts.ui_variable = data
                .variation_axes()
                .iter()
                .any(|axis| axis.tag.to_be_bytes() == *b"wght");
            if !fonts.ui_variable {
                fonts.ui_semibold = find(name, fontdb::Weight::SEMIBOLD)
                    .or_else(|| find(name, fontdb::Weight::BOLD));
            }
            fonts.ui = Some(face);
            break;
        }
        const CJK: [&str; 14] = [
            "Noto Sans CJK TC",
            "Noto Sans CJK JP",
            "Noto Sans CJK SC",
            "Source Han Sans TC",
            "Source Han Sans",
            "PingFang TC",
            "PingFang HK",
            "Heiti TC",
            "Hiragino Sans GB",
            "Hiragino Sans",
            "Hiragino Kaku Gothic ProN",
            "Arial Unicode MS",
            "Microsoft JhengHei",
            "Microsoft YaHei",
        ];
        fonts.cjk = CJK.into_iter().find_map(|name| {
            let families = [fontdb::Family::Name(name)];
            let id = db.query(&fontdb::Query {
                families: &families,
                ..Default::default()
            })?;
            db.with_face_data(id, |data, index| Face {
                bytes: data.to_vec(),
                index,
            })
        });
        // Faces for the symbols the UI face lacks. No single family covers them all (⌞ is in
        // Symbols/Math, ⚠ ★ in Symbols 2/DejaVu), so each installed one is kept only when it
        // adds symbol code points the earlier ones lack. fontdb matches names exactly, hence
        // both spellings of Symbols 2.
        const SYMBOLS: [&str; 8] = [
            "Segoe UI Symbol",
            "Apple Symbols",
            "Noto Sans Symbols",
            "Noto Sans Symbols 2",
            "Noto Sans Symbols2",
            "Noto Sans Math",
            "Symbola",
            "DejaVu Sans",
        ];
        let mut covered = std::collections::HashSet::new();
        for name in SYMBOLS {
            let families = [fontdb::Family::Name(name)];
            let Some(id) = db.query(&fontdb::Query {
                families: &families,
                ..Default::default()
            }) else {
                continue;
            };
            let face = db.with_face_data(id, |data, index| {
                let added = new_symbols(data, index, &covered)?;
                Some((
                    added,
                    Face {
                        bytes: data.to_vec(),
                        index,
                    },
                ))
            });
            if let Some(Some((added, face))) = face {
                covered.extend(added);
                fonts.symbols.push(face);
            }
        }
        fonts
    })
}

/// The symbol code points (General Punctuation to Misc Symbols and Arrows, and the emoji
/// planes) that the face at `index` in `data` outlines and `covered` lacks. `None` when
/// there are none, or the face has no outline tables (bitmap-only color emoji paints
/// nothing in egui).
#[cfg(not(target_arch = "wasm32"))]
fn new_symbols(
    data: &[u8],
    index: u32,
    covered: &std::collections::HashSet<u32>,
) -> Option<Vec<u32>> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    let tables = face.tables();
    if tables.glyf.is_none() && tables.cff.is_none() && tables.cff2.is_none() {
        return None;
    }
    let mut added = Vec::new();
    for subtable in tables.cmap?.subtables {
        if !subtable.is_unicode() {
            continue;
        }
        subtable.codepoints(|cp| {
            // A face has several Unicode subtables that repeat code points.
            if matches!(cp, 0x2000..=0x2BFF | 0x1F000..=0x1FAFF)
                && !covered.contains(&cp)
                && !added.contains(&cp)
            {
                added.push(cp);
            }
        });
    }
    (!added.is_empty()).then_some(added)
}

/// The platform's UI face in front of egui's own (with a [`SEMIBOLD`] variant), an installed
/// CJK-capable font so Chinese / Japanese layer names render, and symbol faces (⌞ ⚠ ★) last.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_fonts(ctx: &egui::Context) {
    use std::sync::Arc;

    let system = system_fonts();
    let mut fonts = egui::FontDefinitions::default();
    let mut semibold = None;
    if let Some(ui) = &system.ui {
        let regular = ui.data(system.ui_variable.then_some(400.0));
        fonts.font_data.insert("ui".into(), Arc::new(regular));
        if let Some(names) = fonts.families.get_mut(&FontFamily::Proportional) {
            names.insert(0, "ui".into());
        }
        let bold = match (&system.ui_semibold, system.ui_variable) {
            (_, true) => Some(ui.data(Some(600.0))),
            (Some(face), false) => Some(face.data(None)),
            (None, false) => None,
        };
        if let Some(bold) = bold {
            fonts.font_data.insert("ui-semibold".into(), Arc::new(bold));
            semibold = Some("ui-semibold");
        }
    }
    add_semibold_family(&mut fonts, semibold);
    let mut fallbacks = |name: &str, monospace: bool| {
        let families = [
            FontFamily::Proportional,
            FontFamily::Name(SEMIBOLD.into()),
            FontFamily::Monospace,
        ];
        for family in &families[..if monospace { 3 } else { 2 }] {
            let family = family.clone();
            let names = fonts.families.entry(family).or_default();
            names.retain(|n| n != name);
            names.push(name.into());
        }
    };
    if let Some(cjk) = &system.cjk {
        fonts
            .font_data
            .insert("cjk".into(), Arc::new(cjk.data(None)));
        fallbacks("cjk", true);
    }
    // After the CJK face: symbol faces for ⌞/⚠/★ that the UI and CJK faces lack. egui's
    // bundled emoji faces stay ahead of them, so plain emoji still come from those. Not on
    // Monospace: these are proportional faces.
    for (i, symbols) in system.symbols.iter().enumerate() {
        let name = format!("symbols-{i}");
        fonts
            .font_data
            .insert(name.clone(), Arc::new(symbols.data(None)));
        fallbacks(&name, false);
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_keeps_its_tint_unless_an_open_file_has_it() {
        let own = pick_tint("design.fig", []);
        assert_eq!(pick_tint("design.fig", []), own, "stable");
        let next = pick_tint("design.fig", [own]);
        assert_ne!(next, own);
        let all: Vec<usize> = (0..TINTS).collect();
        assert_eq!(
            pick_tint("design.fig", all),
            own,
            "all taken: its own again"
        );
        let spread: std::collections::HashSet<usize> = (0..40)
            .map(|i| pick_tint(&format!("file {i}.fig"), []))
            .collect();
        assert!(
            spread.len() >= TINTS - 1,
            "names spread over the tints: {spread:?}"
        );
    }

    #[test]
    fn properties_card_takes_the_space_colour_and_stays_readable() {
        let customs = [
            [0, 0, 0],
            [255, 230, 0],
            [0, 40, 255],
            [255, 0, 0],
            [119, 119, 119],
        ];
        for p in [LIGHT, DARK] {
            let spaces = (0..TINTS)
                .map(Colour::Preset)
                .chain(customs.map(Colour::Custom));
            for colour in spaces {
                let t = space_tint(colour, p.dark);
                let card = p.on_tint(t);
                for bg in [card.card, card.well] {
                    assert!(contrast(p.text, bg) >= 7.0, "{colour:?} dark={}", p.dark);
                    assert!(
                        contrast(p.text_dim, bg) >= 4.5,
                        "{colour:?} dark={}",
                        p.dark
                    );
                    assert!(
                        contrast(p.text_faint, bg) >= 3.0,
                        "{colour:?} dark={}",
                        p.dark
                    );
                }
                // Wells stand apart from the card as much as the neutral ones do.
                assert!(
                    contrast(card.card, card.well) >= contrast(p.card, p.well) - 0.02,
                    "well on card {colour:?} dark={}: {:.3}",
                    p.dark,
                    contrast(card.card, card.well)
                );
            }
            // A preset space shows: the card is no longer the neutral one.
            assert_ne!(p.on_tint(tint(0, p.dark)).card, p.card, "dark={}", p.dark);
        }
    }

    #[test]
    fn text_is_readable_on_every_tint() {
        for p in [LIGHT, DARK] {
            for i in 0..TINTS {
                let t = tint(i, p.dark);
                for bg in [t.top, t.bottom, t.glow] {
                    assert!(
                        contrast(p.text, bg) >= 7.0,
                        "text on tint {i} dark={}",
                        p.dark
                    );
                    assert!(
                        contrast(p.text_dim, bg) >= 4.5,
                        "dim text on tint {i} dark={}",
                        p.dark
                    );
                    assert!(
                        contrast(p.text_faint, bg) >= 3.0,
                        "faint text on tint {i} dark={}",
                        p.dark
                    );
                }
            }
            assert!(contrast(p.text, p.card) >= 7.0);
            assert!(contrast(p.text_faint, p.card) >= 3.0);
        }
    }

    #[test]
    fn any_custom_colour_keeps_text_readable_and_its_accent_visible() {
        let picks = [
            [0, 0, 0],
            [255, 255, 255],
            [255, 230, 0],
            [0, 40, 255],
            [255, 0, 0],
            [0, 200, 120],
            [128, 128, 128],
            // Mid tones, where neither black nor white text reaches 4.5:1 unchanged.
            [68, 136, 85],
            [119, 119, 119],
            [0, 140, 180],
        ];
        for p in [LIGHT, DARK] {
            for rgb in picks {
                let t = space_tint(Colour::Custom(rgb), p.dark);
                for bg in [t.top, t.bottom, t.glow] {
                    assert!(contrast(p.text, bg) >= 7.0, "{rgb:?} dark={}", p.dark);
                    assert!(contrast(p.text_dim, bg) >= 4.5, "{rgb:?} dark={}", p.dark);
                    assert!(contrast(p.text_faint, bg) >= 3.0, "{rgb:?} dark={}", p.dark);
                }
                assert!(
                    contrast(t.accent, p.card) >= 3.0,
                    "accent {rgb:?} dark={}",
                    p.dark
                );
                assert!(
                    contrast(on_accent(t.accent), t.accent) >= 4.5,
                    "button {rgb:?}"
                );
            }
        }
        // A strong colour shows: the gradient is not just the neutral base.
        let blue = space_tint(Colour::Custom([0, 40, 255]), false);
        assert!(blue.bottom.b() > blue.bottom.r() + 10, "{:?}", blue.bottom);
    }

    #[test]
    fn cjk_font_replaces_existing_fallback() {
        let ctx = egui::Context::default();
        // Nothing paints here, so clear the texture deltas (epaint asserts on unapplied ones).
        let frame = |ctx: &egui::Context| {
            ctx.run_ui(Default::default(), |_| {})
                .textures_delta
                .clear();
        };
        frame(&ctx);
        let data: Vec<_> = egui::FontDefinitions::default()
            .font_data
            .values()
            .cloned()
            .collect();
        for font in &data[..2] {
            add_cjk_font(&ctx, font.font.to_vec());
            frame(&ctx);
            ctx.fonts(|fonts| {
                let definitions = fonts.definitions();
                assert_eq!(definitions.font_data["cjk"].font, font.font);
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    let names = &definitions.families[&family];
                    assert_eq!(names.iter().filter(|name| *name == "cjk").count(), 1);
                    assert_eq!(names.last().map(String::as_str), Some("cjk"));
                }
            });
        }
    }

    /// A symbol face is kept only for symbols the earlier faces lack: egui's emoji-icon font
    /// adds U+2605 ★ on top of Noto Emoji, and a second copy adds nothing. Bundled fonts, so
    /// it does not depend on the host.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_symbol_face_is_kept_only_for_symbols_the_earlier_ones_lack() {
        use epaint_default_fonts::{EMOJI_ICON, NOTO_EMOJI_REGULAR};
        let mut covered = std::collections::HashSet::new();
        covered.extend(new_symbols(NOTO_EMOJI_REGULAR, 0, &covered).unwrap());
        assert!(!covered.contains(&0x2605));
        let added = new_symbols(EMOJI_ICON, 0, &covered).unwrap();
        assert!(added.contains(&0x2605), "★ comes from the second face");
        covered.extend(added);
        assert_eq!(new_symbols(EMOJI_ICON, 0, &covered), None, "nothing new");
    }

    #[test]
    fn cjk_detection() {
        for c in ['香', 'ア', 'ㄅ', '，', '！', '「', '한'] {
            assert!(is_cjk(c), "{c}");
        }
        for c in ['a', 'é', 'Ж', '—', '€', '😀'] {
            assert!(!is_cjk(c), "{c}");
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let mut doc = figstudio_fig::Document::open(path).unwrap();
        assert_eq!(cjk_need(&doc, "structure.fig"), CjkNeed::None);
        assert_eq!(cjk_need(&doc, "設計.fig"), CjkNeed::Any, "file name");
        let mut nodes = doc.nodes.clone();
        nodes[3].name = "按鈕".into();
        doc.set_nodes(nodes).unwrap();
        assert_eq!(cjk_need(&doc, "structure.fig"), CjkNeed::Any, "layer name");
    }

    #[test]
    fn hangul_without_other_cjk_asks_for_the_korean_face() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let mut doc = figstudio_fig::Document::open(path).unwrap();
        let need = |doc: &figstudio_fig::Document, name: &str| cjk_need(doc, name);
        assert_eq!(need(&doc, "structure.fig"), CjkNeed::None);
        assert_eq!(
            need(&doc, "「draft」.fig"),
            CjkNeed::Shared,
            "punctuation only"
        );
        for name in [
            "디자인.fig",
            "ㄱ.fig",
            "안녕。.fig",
            "디자인！.fig",
            "한글「텍스트」.fig",
            "디자인\u{3000}.fig",
        ] {
            assert_eq!(need(&doc, name), CjkNeed::HangulOnly, "{name}");
        }
        assert_eq!(need(&doc, "設計.fig"), CjkNeed::Any, "ideographs");
        let mut nodes = doc.nodes.clone();
        nodes[3].name = "버튼".into();
        doc.set_nodes(nodes).unwrap();
        assert_eq!(
            need(&doc, "structure.fig"),
            CjkNeed::HangulOnly,
            "layer name"
        );
        let mut nodes = doc.nodes.clone();
        nodes[4].name = "按鈕".into();
        doc.set_nodes(nodes).unwrap();
        assert_eq!(
            need(&doc, "structure.fig"),
            CjkNeed::Any,
            "ideograph after Hangul"
        );
    }
}
