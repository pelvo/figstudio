//! Agent access controls for cloud execution and the existing opt-in local upload bridge.
use super::*;
impl Host {
    pub fn menu(&mut self, ui: &mut egui::Ui) {
        let current_mode = if self.proxy.is_some() {
            2u8
        } else if self.relay {
            0u8
        } else {
            1u8
        };
        let mut mode = current_mode;
        let cloud_offered = self.cloud.get().offered();
        ui.horizontal(|ui| {
            if cloud_offered {
                ui.selectable_value(&mut mode, 0, "Cloud relay");
            }
            ui.selectable_value(&mut mode, 1, "Local bridge");
        });
        if !cloud_offered && self.proxy.is_none() {
            ui.label("This site has no cloud relay. For an agent on this computer, run");
            ui.label("`figstudio-mcp --browser-proxy` (zero upload) or `--bridge`.");
        }
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
            // Apply first, from the right, so the field takes only the room left: in Settings'
            // narrow column a default-width field pushed the button out of view.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let apply = ui.button("Apply");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.room_input)
                            .password(true)
                            .hint_text("executor token or room link")
                            .desired_width(ui.available_width()),
                    );
                    if apply.clicked() {
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
            if self.relay && self.proxy.is_none() && self.cloud_gate.waiting() {
                ui.label("Checking that this site runs the cloud relay…");
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
        ui.label(RichText::new(text).color(theme::of(ui).text_dim));
        ui.separator();
    }
}
