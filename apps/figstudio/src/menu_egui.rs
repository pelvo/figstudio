//! Drawn menu for Linux, web, headless tests and native-attach fallback.
use crate::{
    APP_NAME, App,
    menu::{Action, Item},
    theme,
};

impl App {
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let tree = crate::menu::tree(self);
        egui::MenuBar::new().ui(ui, |ui| {
            for group in tree {
                ui.menu_button(group.label, |ui| self.menu_items(ui, &group.children));
            }
        });
    }

    fn menu_items(&mut self, ui: &mut egui::Ui, items: &[Item]) {
        for item in items {
            if item.label.is_empty() {
                ui.separator();
                continue;
            }
            let Some(action) = item.action else {
                ui.add_enabled_ui(item.enabled, |ui| {
                    ui.menu_button(item.label, |ui| self.menu_items(ui, &item.children));
                });
                continue;
            };
            #[cfg(target_arch = "wasm32")]
            if let Action::CjkFont(v) = action {
                if ui.radio(item.checked == Some(true), item.label).clicked() {
                    self.apply(action, ui.ctx());
                }
                if self.web.korean_override && v == *crate::cjk::Variant::ALL.last().unwrap_or(&v) {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Korean (KR) in use: a file has only Hangul")
                            .color(theme::TEXT_DIM),
                    );
                }
                continue;
            }
            let clicked = if let Some(mut checked) = item.checked {
                ui.add_enabled(item.enabled, egui::Checkbox::new(&mut checked, item.label))
                    .clicked()
            } else {
                let mut button = egui::Button::new(item.label);
                if matches!(action, Action::Open | Action::ExportSelection(2.0))
                    && let Some(shortcut) = action.shortcut()
                {
                    button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
                }
                ui.add_enabled(item.enabled, button).clicked()
            };
            if clicked {
                ui.close();
                self.apply(action, ui.ctx());
            }
        }
    }

    pub(crate) fn help_text(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(format!("{APP_NAME} {}", env!("CARGO_PKG_VERSION"))).strong());
        ui.label("Opens Figma .fig local copies offline.");
        ui.label(
            egui::RichText::new("V select · H hand · Z zoom · Space drag to pan")
                .color(theme::TEXT_DIM),
        );
        ui.label(
            egui::RichText::new("Ctrl+scroll or pinch to zoom · ⇧1 fit").color(theme::TEXT_DIM),
        );
    }
}
