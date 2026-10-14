//! Settings and help belong in windows, not editable widgets inside menus.
use crate::App;

impl App {
    pub(crate) fn menu_windows(&mut self, ctx: &egui::Context) {
        egui::Window::new("Agent access")
            .open(&mut self.agent_window)
            .resizable(false)
            .show(ctx, |ui| {
                self.mcp.menu(ui);
                self.mcp.status(ui);
            });
        let mut open = self.about_window;
        egui::Window::new("About figstudio")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| self.help_text(ui));
        self.about_window = open;
    }
}
