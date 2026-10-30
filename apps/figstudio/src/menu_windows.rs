//! Windows of the app: Settings (agent access and About live in it). A window, not
//! editable widgets inside menus.
use crate::App;

impl App {
    pub(crate) fn menu_windows(&mut self, ctx: &egui::Context) {
        self.settings_window(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.update_notes(ctx);
        self.link_window(ctx);
    }

    /// The update popup is open (desktop): Escape and plain keys are its.
    pub(crate) fn update_notes_up(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.updater.notes_open;
        #[cfg(target_arch = "wasm32")]
        false
    }
}
