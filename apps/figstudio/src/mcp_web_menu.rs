//! Agent access controls for cloud execution and the existing opt-in local upload bridge.
use super::*;
impl McpHost {
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        let previous = self.relay;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.relay, true, "Cloud relay");
            ui.selectable_value(&mut self.relay, false, "Local bridge");
        });
        if self.relay != previous {
            self.stop();
        }
        if self.relay {
            ui.label("MCP tools run in this browser; files are not uploaded.");
            ui.label("The MCP URL grants access to open files. Keep it private.");
            ui.label("Calls go to the browser tab that connected or changed most recently.");
            if ui.button("Copy MCP URL").clicked()
                && let Some((_, url)) = room::urls(&self.room)
            {
                ui.ctx().copy_text(url);
            }
            if ui.button("Regenerate room token").clicked() {
                match room::regenerate() {
                    Ok(room) => {
                        self.stop();
                        self.room = room;
                        self.want_connect = true;
                    }
                    Err(error) => self.notices.push(error),
                }
            }
        } else {
            ui.label("Pairs this tab with `figstudio-mcp --bridge` on this computer.");
            ui.add(egui::TextEdit::singleline(&mut self.url).hint_text(DEFAULT_URL));
            ui.add(
                egui::TextEdit::singleline(&mut self.token)
                    .password(true)
                    .hint_text("pairing token"),
            );
        }
        if self.conn.is_none() {
            if ui.button("Connect").clicked() {
                self.want_connect = true;
            }
        } else if ui.button("Disconnect").clicked() {
            self.stop();
        }
        if self.conn.as_ref().is_some_and(|c| c.welcomed) {
            ui.label(self.connected_label());
        }
    }

    fn connected_label(&self) -> &'static str {
        if self.relay {
            "Connected to cloud relay"
        } else {
            "Connected to local bridge"
        }
    }

    pub fn status(&self, ui: &mut egui::Ui) {
        let Some(conn) = &self.conn else { return };
        let text = match (conn.welcomed, self.shared.progress.get()) {
            (false, _) => "MCP connecting…".to_owned(),
            (true, Some((sent, total))) => format!("MCP uploading {}%", sent * 100 / total.max(1)),
            (true, None) => self.connected_label().to_owned(),
        };
        ui.label(RichText::new(text).color(theme::TEXT_DIM));
        ui.separator();
    }
}
