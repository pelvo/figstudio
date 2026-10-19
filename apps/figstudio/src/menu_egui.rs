//! Drawn menu for Linux, web, headless tests and native-attach fallback.
use crate::{APP_NAME, App, menu::Item, theme};

impl App {
    /// The "⋯" button in the sidebar: the File, View and Help menus where there is no native
    /// menu bar (Linux, the browser, tests).
    pub(crate) fn menu_button(&mut self, ui: &mut egui::Ui) {
        // No tooltip: egui's tooltip on the same response would keep the menu from opening.
        let response = crate::icons::raw_button(ui, crate::icons::Icon::More, "Menu", false, 28.0);
        let tree = crate::menu::tree(self);
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(170.0);
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
            if let crate::menu::Action::CjkFont(v) = action {
                if ui.radio(item.checked == Some(true), item.label).clicked() {
                    self.apply(action, ui.ctx());
                }
                if self.web.korean_override && v == *crate::cjk::Variant::ALL.last().unwrap_or(&v) {
                    ui.separator();
                    ui.label(
                        egui::RichText::new("Korean (KR) in use: a file has only Hangul")
                            .color(theme::of(ui).text_dim),
                    );
                }
                continue;
            }
            let clicked = if let Some(mut checked) = item.checked {
                let response =
                    ui.add_enabled(item.enabled, egui::Checkbox::new(&mut checked, item.label));
                if let Some(shortcut) = action.shortcut() {
                    response
                        .on_hover_text(ui.ctx().format_shortcut(&shortcut))
                        .clicked()
                } else {
                    response.clicked()
                }
            } else {
                let mut button = egui::Button::new(item.label);
                // Labels with a typed hint ("Zoom to fit   ⇧1") already show it.
                if !item.label.contains("   ")
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
                .color(theme::of(ui).text_dim),
        );
        ui.label(
            egui::RichText::new("Ctrl+scroll or pinch to zoom · ⇧1 fit")
                .color(theme::of(ui).text_dim),
        );
        let bar = ui.ctx().format_shortcut(&crate::command::SHORTCUT);
        ui.label(egui::RichText::new(format!("{bar} command bar")).color(theme::of(ui).text_dim));
    }
}
