//! Agent access when the `mcp` feature is off (desktop): no menu item, no thread, no socket.
//! Same API as `mcp_host.rs`.
use crate::tab::Tab;

#[derive(Default)]
pub struct McpHost;

impl McpHost {
    pub fn access(&mut self) -> Option<crate::settings::Access<'_>> {
        None
    }

    pub fn menu(&mut self, ui: &mut egui::Ui) {
        ui.label("This build has no agent access (built without the mcp feature).");
    }

    pub fn status(&self, _ui: &mut egui::Ui) {}

    pub fn confirm(&mut self, _ctx: &egui::Context) {}

    pub fn frame(&mut self, _tabs: &mut [Tab], _active: &mut usize) -> Option<String> {
        None
    }

    pub fn take_links(&mut self) -> Vec<crate::link_open::LinkAsk> {
        Vec::new()
    }

    pub fn sync(&mut self, _ctx: &egui::Context, _tabs: &[Tab], _active: usize) {}

    /// [`Self::sync`], then send the link answers: an MCP caller told a file is open finds
    /// it in the published snapshot.
    pub fn sync_then_answer(
        &mut self,
        ctx: &egui::Context,
        tabs: &[Tab],
        active: usize,
        answers: Vec<crate::link_open::Answer>,
    ) {
        self.sync(ctx, tabs, active);
        for (done, result) in answers {
            done(result);
        }
    }

    pub fn stop(&mut self) {}
}
