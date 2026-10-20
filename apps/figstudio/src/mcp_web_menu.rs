//! Agent access controls for cloud execution and the existing opt-in local upload bridge.
use super::*;
impl McpHost {
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        let current_mode = if self.proxy.is_some() {
            2u8
        } else if self.relay {
            0u8
        } else {
            1u8
        };
        let mut mode = current_mode;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut mode, 0, "Cloud relay");
            ui.selectable_value(&mut mode, 1, "Local bridge");
        });
        if mode != current_mode {
            self.proxy = None;
            self.relay = mode == 0;
            self.stop();
        }
        if self.proxy.is_some() {
            ui.label("Paired with local `figstudio-mcp --browser-proxy` (zero upload).");
            ui.label("MCP tools run in this browser; files are not uploaded.");
            ui.horizontal(|ui| {
                ui.label("Device label");
                let id = egui::Id::new("mcp_device_label_input");
                if !ui.ctx().memory(|m| m.has_focus(id))
                    && let Some(saved) = room::raw_stored_device_label()
                {
                    self.device_label = saved;
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.device_label)
                            .id(id)
                            .char_limit(64)
                            .hint_text("e.g. Office · Chrome"),
                    )
                    .changed()
                {
                    room::save_device_label(&self.device_label);
                    self.last_key = None;
                }
            });
        } else if self.relay {
            ui.label("MCP tools run in this browser; files are not uploaded.");
            ui.label("Anyone with the MCP URL can call the tools; the room link also");
            ui.label("grants executor authority over the documents open in this browser.");
            ui.label("Use list_sessions / select_session when multiple devices share a room.");
            ui.horizontal(|ui| {
                ui.label("Device label");
                let id = egui::Id::new("mcp_device_label_input");
                if !ui.ctx().memory(|m| m.has_focus(id))
                    && let Some(saved) = room::raw_stored_device_label()
                {
                    self.device_label = saved;
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.device_label)
                            .id(id)
                            .char_limit(64)
                            .hint_text("e.g. Office · Chrome"),
                    )
                    .changed()
                {
                    room::save_device_label(&self.device_label);
                    self.last_key = None;
                }
            });
            ui.horizontal(|ui| {
                let room = self.pair.as_ref().map(|p| p.room());
                if ui.button("Copy MCP URL").clicked()
                    && let Some(room) = &room
                    && let Some((_, url)) = room::urls(room)
                {
                    ui.ctx().copy_text(url);
                }
                if ui.button("Copy room link").clicked()
                    && let Some(pair) = &self.pair
                    && let Some(link) = room::link(pair)
                {
                    ui.ctx().copy_text(link);
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.room_input)
                        .password(true)
                        .hint_text("executor token or room link"),
                );
                if ui.button("Apply").clicked() {
                    match room::join(&self.room_input) {
                        Ok(pair) => {
                            // Typing or pasting a token here is already consent.
                            self.adopt(pair);
                            self.room_input.clear();
                        }
                        // An invalid entry changes nothing.
                        Err(error) => self.notices.push(error),
                    }
                }
            });
            if ui.button("Regenerate room token").clicked() {
                // Saved before the old connection is dropped (`regenerate` persists first):
                // a storage failure keeps the current room working.
                match room::regenerate() {
                    Ok(pair) => self.adopt(pair),
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
        if self.relay && self.proxy.is_some() {
            "Connected to local browser proxy"
        } else if self.relay {
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
