//! Arc-style look: soft per-file gradients behind floating cards, in light and dark.
//!
//! The window background is a gradient tinted per open file ([`Tint`], like Arc spaces). The
//! sidebar sits straight on it with translucent rows; the canvas and the properties panel are
//! opaque rounded cards. [`Palette`] holds the colours of one appearance; the app follows the
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
    text_faint: Color32::from_rgb(128, 128, 140),
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
    text_faint: Color32::from_rgb(120, 120, 132),
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
        fonts
    })
}

/// The platform's UI face in front of egui's own (with a [`SEMIBOLD`] variant), and an
/// installed CJK-capable font as the last fallback so Chinese / Japanese layer names render.
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
    if let Some(cjk) = &system.cjk {
        fonts
            .font_data
            .insert("cjk".into(), Arc::new(cjk.data(None)));
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(SEMIBOLD.into()),
        ] {
            fonts.families.entry(family).or_default().push("cjk".into());
        }
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
    fn text_is_readable_on_every_tint() {
        // WCAG contrast of two sRGB colours.
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
        fn contrast(a: Color32, b: Color32) -> f32 {
            let (a, b) = (luminance(a), luminance(b));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        }
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
                }
            }
            assert!(contrast(p.text, p.card) >= 7.0);
            assert!(contrast(p.text_faint, p.card) >= 3.0);
        }
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
