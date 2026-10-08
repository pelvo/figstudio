//! Dark "pro" theme: neutral greys and a blue accent, laid out like PhotoCraft / Photoshop.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke, Visuals};

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

/// Adds an installed CJK-capable font as a fallback so Chinese / Japanese layer names render.
pub fn install_fonts(ctx: &egui::Context) {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let wanted = [
        "Noto Sans CJK TC",
        "Noto Sans CJK JP",
        "Noto Sans CJK SC",
        "Source Han Sans TC",
        "Source Han Sans",
        "PingFang TC",
        "Hiragino Sans",
        "Hiragino Kaku Gothic ProN",
        "Microsoft JhengHei",
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
