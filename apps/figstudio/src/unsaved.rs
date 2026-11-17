//! Asks before closing a file, or quitting, would drop edits that are in no file.

use crate::App;

/// A close waiting for the user's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Closing the tab with this id.
    Tab(u64),
    /// Quitting the app.
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
    Cancel,
    Discard,
    SaveCopy,
}

impl App {
    /// Closes tab `i`, first asking when it has unsaved edits.
    pub(crate) fn close(&mut self, i: usize) {
        match self.tabs.get(i) {
            Some(t) if t.dirty_unsaved => self.unsaved = Some(Pending::Tab(t.id)),
            Some(_) => self.close_now(i),
            None => {}
        }
    }

    /// Quits, first asking when any tab has unsaved edits.
    pub(crate) fn quit(&mut self, ctx: &egui::Context) {
        if self.quit_confirmed || !self.any_unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.unsaved = Some(Pending::Quit);
        }
    }

    fn any_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.dirty_unsaved)
    }

    /// Shows the question while a close waits, and carries out the answer. Also holds a
    /// window close from the OS (title bar, Dock) while edits are unsaved.
    pub(crate) fn unsaved_confirm(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_confirmed
            && self.any_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.unsaved = Some(Pending::Quit);
        }
        let Some(pending) = self.unsaved else {
            return;
        };
        let (title, body, discard) = match pending {
            Pending::Tab(id) => {
                let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
                    self.unsaved = None;
                    return;
                };
                (
                    format!("Close “{}” without saving?", tab.title()),
                    "Its edits are not in any file yet.".to_string(),
                    "Discard edits",
                )
            }
            Pending::Quit => {
                let n = self.tabs.iter().filter(|t| t.dirty_unsaved).count();
                let files = if n == 1 { "file has" } else { "files have" };
                (
                    "Quit without saving?".to_string(),
                    format!(
                        "{n} open {files} edits that are not in any file yet. Cancel to save a copy first."
                    ),
                    "Quit without saving",
                )
            }
        };
        let mut reply = None;
        let modal = egui::Modal::new(egui::Id::new("unsaved-edits")).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(title);
            ui.label(body);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancel");
                if !ui.ctx().memory(|m| m.focused().is_some()) {
                    cancel.request_focus();
                }
                if cancel.clicked() {
                    reply = Some(Reply::Cancel);
                }
                if ui.button(discard).clicked() {
                    reply = Some(Reply::Discard);
                }
                if matches!(pending, Pending::Tab(_)) && ui.button("Save a copy…").clicked() {
                    reply = Some(Reply::SaveCopy);
                }
            });
        });
        if reply.is_none() && modal.should_close() {
            reply = Some(Reply::Cancel);
        }
        let Some(reply) = reply else {
            return;
        };
        self.unsaved = None;
        match (pending, reply) {
            (_, Reply::Cancel) => {}
            (Pending::Quit, _) => {
                self.quit_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            (Pending::Tab(id), Reply::Discard) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    self.close_now(i);
                }
            }
            (Pending::Tab(id), Reply::SaveCopy) => {
                if let Some(i) = self.tabs.iter().position(|t| t.id == id) {
                    self.active = i;
                    // A cancelled or failed save keeps the tab open.
                    if self.save_copy(ctx)
                        && let Some(i) = self.tabs.iter().position(|t| t.id == id)
                    {
                        self.close_now(i);
                    }
                }
            }
        }
    }
}
