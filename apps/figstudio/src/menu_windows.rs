//! Windows of the app: Settings (agent access and About live in it). A window, not
//! editable widgets inside menus.
use crate::App;

impl App {
    pub(crate) fn menu_windows(&mut self, ctx: &egui::Context) {
        self.settings_window(ctx);
    }
}
