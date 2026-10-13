//! Dark "pro" theme: neutral greys and a blue accent, laid out like PhotoCraft / Photoshop.

use egui::{Color32, CornerRadius, Stroke, Visuals};

pub const CHROME: Color32 = Color32::from_rgb(50, 50, 50);
pub const DOCK: Color32 = Color32::from_rgb(38, 38, 38);
pub const CANVAS: Color32 = Color32::from_rgb(30, 30, 30);
pub const CARD: Color32 = Color32::from_rgb(50, 50, 50);
pub const FIELD: Color32 = Color32::from_rgb(36, 36, 36);
pub const SEPARATOR: Color32 = Color32::from_rgb(26, 26, 26);
pub const TAB_STRIP: Color32 = Color32::from_rgb(38, 38, 38);
pub const HOVER: Color32 = Color32::from_rgb(66, 66, 66);
pub const ROW_SELECTED: Color32 = Color32::from_rgb(55, 85, 125);
pub const TEXT: Color32 = Color32::from_rgb(222, 222, 222);
pub const TEXT_DIM: Color32 = Color32::from_rgb(170, 170, 170);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(120, 120, 120);
pub const ACCENT: Color32 = Color32::from_rgb(55, 142, 240);
pub const WARNING: Color32 = Color32::from_rgb(232, 176, 70);

pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = DOCK;
    v.window_fill = CARD;
    v.extreme_bg_color = FIELD;
    v.faint_bg_color = Color32::from_rgb(44, 44, 44);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ROW_SELECTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, SEPARATOR);
    let r = CornerRadius::same(3);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SEPARATOR);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(60, 60, 60);
    v.widgets.inactive.bg_fill = Color32::from_rgb(60, 60, 60);
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.active.weak_bg_fill = Color32::from_rgb(78, 78, 78);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
        s.spacing.interact_size.y = 20.0;
    });
}

/// Browser build: nothing at start-up (no installed fonts to read). A CJK font is fetched
/// when a file needs one (`web::Inbox::load_cjk_font`, after [`cjk_need`]).
#[cfg(target_arch = "wasm32")]
pub fn install_fonts(_ctx: &egui::Context) {}

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

/// What the CJK fallback has to draw for a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CjkNeed {
    None,
    /// Hangul and no other CJK: only the KR face draws it.
    HangulOnly,
    Any,
}

impl CjkNeed {
    fn add(self, text: &str) -> Self {
        text.chars().fold(self, |need, c| match need {
            CjkNeed::Any => CjkNeed::Any,
            _ if is_hangul(c) => CjkNeed::HangulOnly,
            _ if is_cjk(c) => CjkNeed::Any,
            need => need,
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
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let names = fonts.families.entry(family).or_default();
        names.retain(|name| name != "cjk");
        names.push("cjk".into());
    }
    ctx.set_fonts(fonts);
}

/// Adds an installed CJK-capable font as a fallback so Chinese / Japanese layer names render.
#[cfg(not(target_arch = "wasm32"))]
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions, FontFamily};

    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let wanted = [
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
    let mut fonts = FontDefinitions::default();
    for name in wanted {
        let families = [fontdb::Family::Name(name)];
        let Some(id) = db.query(&fontdb::Query {
            families: &families,
            ..Default::default()
        }) else {
            continue;
        };
        let Some((bytes, index)) = db.with_face_data(id, |data, index| (data.to_vec(), index))
        else {
            continue;
        };
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        fonts
            .font_data
            .insert("cjk".into(), std::sync::Arc::new(data));
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("cjk".into());
        }
        break;
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_font_replaces_existing_fallback() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |_| {});
        let data: Vec<_> = egui::FontDefinitions::default()
            .font_data
            .values()
            .cloned()
            .collect();
        for font in &data[..2] {
            add_cjk_font(&ctx, font.font.to_vec());
            let _ = ctx.run_ui(Default::default(), |_| {});
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
        assert_eq!(
            cjk_need(&doc, "디자인.fig"),
            CjkNeed::HangulOnly,
            "file name"
        );
        let mut nodes = doc.nodes.clone();
        nodes[3].name = "버튼".into();
        doc.set_nodes(nodes).unwrap();
        assert_eq!(
            cjk_need(&doc, "structure.fig"),
            CjkNeed::HangulOnly,
            "layer name"
        );
        assert_eq!(
            cjk_need(&doc, "設計.fig"),
            CjkNeed::Any,
            "Hangul and ideographs"
        );
        assert_eq!(
            cjk_need(&doc, "ㄱ.fig"),
            CjkNeed::HangulOnly,
            "compatibility jamo"
        );
    }
}
