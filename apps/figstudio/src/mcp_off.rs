//! Agent access when the `mcp` feature is off (desktop): no menu item, no thread, no socket.
//! Same API as `mcp_host.rs`.
use crate::tab::Tab;

#[derive(Default)]
pub struct McpHost;

impl McpHost {
    pub fn menu(&mut self, _ui: &mut egui::Ui) {}

    pub fn status(&self, _ui: &mut egui::Ui) {}

    pub fn frame(&mut self, _tabs: &mut [Tab], _active: &mut usize) -> Option<String> {
        None
    }

    pub fn sync(&mut self, _ctx: &egui::Context, _tabs: &[Tab], _active: usize) {}

    pub fn stop(&mut self) {}
}
