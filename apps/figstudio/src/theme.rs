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
/// when a file needs one (`web::Inbox::load_cjk_font`, after [`needs_cjk`]).
#[cfg(target_arch = "wasm32")]
pub fn install_fonts(_ctx: &egui::Context) {}

/// Whether `c` needs a CJK font (egui's built-in fonts cover Latin, Greek and Cyrillic).
pub fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{2E80}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FE30}'..='\u{FE4F}'
        | '\u{FF00}'..='\u{FFEF}')
}

/// Whether anything the UI shows for this document (file name, page and layer names, text
/// contents) has CJK characters.
pub fn needs_cjk(doc: &figstudio_fig::Document, file_name: &str) -> bool {
    file_name.chars().any(is_cjk)
        || doc
            .file_name
            .as_deref()
            .is_some_and(|n| n.chars().any(is_cjk))
        || doc.nodes.iter().any(|n| {
            n.name.chars().any(is_cjk)
                || n.data
                    .path(&["textData", "characters"])
                    .and_then(figstudio_fig::Val::as_str)
                    .is_some_and(|t| t.chars().any(is_cjk))
        })
}

/// Adds `bytes` (a CJK font) as the last fallback of every family.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn add_cjk_font(ctx: &egui::Context, bytes: Vec<u8>) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
    let families = [egui::FontFamily::Proportional, egui::FontFamily::Monospace]
        .into_iter()
        .map(|family| InsertFontFamily {
            family,
            priority: FontPriority::Lowest,
        })
        .collect();
    ctx.add_font(FontInsert::new(
        "cjk",
        egui::FontData::from_owned(bytes),
        families,
    ));
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
    fn cjk_detection() {
        for c in ['香', 'ア', 'ㄅ', '，', '！', '「'] {
            assert!(is_cjk(c), "{c}");
        }
        for c in ['a', 'é', 'Ж', '—', '€', '😀'] {
            assert!(!is_cjk(c), "{c}");
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/structure.fig");
        let mut doc = figstudio_fig::Document::open(path).unwrap();
        assert!(!needs_cjk(&doc, "structure.fig"));
        assert!(needs_cjk(&doc, "設計.fig"), "file name");
        let mut nodes = doc.nodes.clone();
        nodes[3].name = "按鈕".into();
        doc.set_nodes(nodes).unwrap();
        assert!(needs_cjk(&doc, "structure.fig"), "layer name");
    }
}
