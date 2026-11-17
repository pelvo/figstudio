//! Asks before closing a file, or quitting, would drop edits that are in no file.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use crate::App;
#[cfg(not(target_arch = "wasm32"))]
use crate::update;

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
        self.quit_for(ctx, cfg!(target_arch = "wasm32"));
    }

    /// Quits on desktop; no-op in the browser build where `ViewportCommand::Close` is ignored.
    pub(crate) fn quit_for(&mut self, ctx: &egui::Context, browser: bool) {
        if browser {
            return;
        }
        if self.quit_confirmed || !self.any_unsaved() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.unsaved = Some(Pending::Quit);
        }
    }

    /// Restarts into the updated bundle, first asking when any tab has unsaved edits: a
    /// restart quits, so it drops them like a quit does.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn restart(&mut self, bundle: PathBuf, ctx: &egui::Context) {
        if !self.quit_confirmed && self.any_unsaved() {
            self.restart_after_quit = Some(bundle);
            self.unsaved = Some(Pending::Quit);
            return;
        }
        match update::relaunch(&bundle) {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(e) => self.toast(ctx, format!("{e:#}"), true),
        }
    }

    /// Whether any open tab has edits that are not saved to its file.
    pub(crate) fn any_unsaved(&self) -> bool {
        self.tabs.iter().any(|t| t.dirty_unsaved)
    }

    /// Shows the question while a close waits, and carries out the answer. Also holds a
    /// window close from the OS (title bar, Dock) while edits are unsaved. Returns whether the
    /// question is drawn this frame.
    pub(crate) fn unsaved_confirm(&mut self, ctx: &egui::Context) -> bool {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.quit_confirmed
            && self.any_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.unsaved = Some(Pending::Quit);
        }
        let Some(pending) = self.unsaved else {
            return false;
        };
        let (title, body, discard) = match pending {
            Pending::Tab(id) => {
                let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
                    self.unsaved = None;
                    return false;
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
            return true;
        };
        self.unsaved = None;
        #[cfg(not(target_arch = "wasm32"))]
        let restart = self.restart_after_quit.take();
        match (pending, reply) {
            (_, Reply::Cancel) => {}
            (Pending::Quit, _) => {
                if cfg!(target_arch = "wasm32") {
                    return true;
                }
                // A restart that cannot be scheduled does not quit (as without edits).
                #[cfg(not(target_arch = "wasm32"))]
                if let Some(bundle) = restart
                    && let Err(e) = update::relaunch(&bundle)
                {
                    self.toast(ctx, format!("{e:#}"), true);
                    return true;
                }
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
        true
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "unsaved_tests.rs"]
mod tests;
