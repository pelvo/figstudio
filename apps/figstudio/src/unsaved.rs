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

/// Document state identity captured when a Save a copy starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SaveSnapshot {
    pub(crate) tab: u64,
    pub(crate) state_id: u64,
    pub(crate) revision: u64,
}

impl SaveSnapshot {
    pub(crate) fn of(tab: &crate::tab::Tab) -> Self {
        Self {
            tab: tab.id,
            state_id: tab.history.state_id(),
            revision: tab.doc.revision,
        }
    }

    pub(crate) fn matches(self, tab: &crate::tab::Tab) -> bool {
        self.tab == tab.id
            && self.state_id == tab.history.state_id()
            && self.revision == tab.doc.revision
    }
}

/// An asynchronous Save a copy currently running for one tab.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InFlightSave {
    pub(crate) id: u64,
    pub(crate) snapshot: SaveSnapshot,
    pub(crate) close_after: bool,
}

/// How an existing in-flight async save arbitrates another Save a copy request.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InFlightAction {
    /// The in-flight save already matches this exact tab snapshot; close intent was merged.
    AlreadyMatching,
    /// A different tab or an older snapshot is currently saving.
    Busy,
}

/// Outcome of a synchronous or asynchronous Save a copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveOutcome {
    /// Saved to a chosen file path or File System Access handle.
    Saved(String),
    /// Requested a browser download of the copy (unconfirmed completion).
    #[cfg(any(target_arch = "wasm32", test))]
    Downloaded(String),
    /// User cancelled the save dialog/picker; nothing was written.
    Cancelled,
    /// Save failed before committing; existing destination (if any) stays untouched.
    Err(String),
}

/// Comparison result between an open tab's source handle and a chosen browser save target.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DestinationIdentity {
    /// Destination is proven to be the open source file.
    Same,
    /// Destination is proven to be a different file from the open source.
    Different,
    /// Source identity is unavailable or comparison failed; cannot prove destination is distinct.
    Unknown,
}

/// Save-target policy for a browser File System Access handle returned by `showSaveFilePicker`.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HandleSaveDecision {
    /// Destination is proven distinct from the open source: safe to call `createWritable()`.
    WriteHandle,
    /// Destination matches the open source: refuse without opening a writable stream.
    RejectSameSource,
    /// Destination identity cannot be proven distinct: fall back to unconfirmed download without
    /// opening a writable stream.
    FallbackDownload,
}

#[cfg(any(target_arch = "wasm32", test))]
impl DestinationIdentity {
    /// Parses the 3-state tag returned by `js/save.js:compareHandles`. Any missing, failed, or
    /// unrecognized result fails closed to [`Self::Unknown`].
    pub(crate) fn from_js_tag(tag: Option<&str>) -> Self {
        match tag {
            Some("same") => Self::Same,
            Some("different") => Self::Different,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn decide(self) -> HandleSaveDecision {
        match self {
            Self::Same => HandleSaveDecision::RejectSameSource,
            Self::Different => HandleSaveDecision::WriteHandle,
            Self::Unknown => HandleSaveDecision::FallbackDownload,
        }
    }
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
            self.updater.notes_open = false;
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
        let id = egui::Id::new("unsaved-edits");
        // Strictly above every Order::Foreground overlay (Command Bar, Settings, Welcome,
        // Space Popover, peek sidebar, tips) so nothing can render over or steal hits from it.
        let area = egui::Modal::default_area(id).order(egui::Order::Debug);
        let modal = egui::Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_max_width(360.0);
            ui.heading(title);
            ui.label(body);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let cancel = ui.button("Cancel");
                if cancel.clicked() {
                    reply = Some(Reply::Cancel);
                }
                let discard_btn = ui.button(discard);
                if discard_btn.clicked() {
                    reply = Some(Reply::Discard);
                }
                let save_btn =
                    matches!(pending, Pending::Tab(_)).then(|| ui.button("Save a copy…"));
                if save_btn.as_ref().is_some_and(egui::Response::clicked) {
                    reply = Some(Reply::SaveCopy);
                }
                let focused = ui.ctx().memory(|m| m.focused());
                let inside = focused.is_some_and(|f| {
                    f == cancel.id
                        || f == discard_btn.id
                        || save_btn.as_ref().is_some_and(|b| f == b.id)
                });
                if !inside {
                    cancel.request_focus();
                }
            });
        });
        ctx.move_to_top(modal.response.layer_id);
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
                    // Routes both synchronous and asynchronous saves through `finish_save_copy`.
                    self.save_copy_for(ctx, true);
                }
            }
        }
        true
    }

    /// If an async save is already running, upgrades its close intent when it matches
    /// `snapshot` or reports `Busy` without touching the running save.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn check_in_flight_save(
        &mut self,
        snapshot: SaveSnapshot,
        close_after: bool,
    ) -> Option<InFlightAction> {
        let in_flight = self.save_in_flight.as_mut()?;
        if in_flight.snapshot == snapshot {
            if close_after {
                in_flight.close_after = true;
            }
            Some(InFlightAction::AlreadyMatching)
        } else {
            Some(InFlightAction::Busy)
        }
    }

    /// Begins tracking an asynchronous Save a copy for `snapshot`, or reports how an existing
    /// in-flight save handles the request.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn begin_async_save_copy(
        &mut self,
        snapshot: SaveSnapshot,
        close_after: bool,
    ) -> Result<u64, InFlightAction> {
        if let Some(action) = self.check_in_flight_save(snapshot, close_after) {
            return Err(action);
        }
        let id = self.next_save_id;
        self.next_save_id = self.next_save_id.wrapping_add(1);
        self.save_in_flight = Some(InFlightSave {
            id,
            snapshot,
            close_after,
        });
        Ok(id)
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn handle_in_flight_save_action(
        &mut self,
        ctx: &egui::Context,
        snapshot: SaveSnapshot,
        close_after: bool,
        action: InFlightAction,
    ) {
        match action {
            InFlightAction::AlreadyMatching => self.toast(ctx, "Saving copy…", false),
            InFlightAction::Busy => self.finish_save_copy(
                ctx,
                snapshot,
                close_after,
                SaveOutcome::Err("a save is already running".into()),
            ),
        }
    }

    /// Completes an asynchronous Save a copy identified by `save_id`, routing its outcome
    /// through [`App::finish_save_copy`]. Stale completions are ignored.
    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn complete_async_save_copy(
        &mut self,
        ctx: &egui::Context,
        save_id: u64,
        outcome: SaveOutcome,
    ) {
        let Some(job) = self.save_in_flight.filter(|j| j.id == save_id) else {
            return;
        };
        self.save_in_flight = None;
        self.finish_save_copy(ctx, job.snapshot, job.close_after, outcome);
    }

    /// Unified completion path for synchronous and asynchronous Save a copy operations.
    /// A normal Save a copy stays a copy (`dirty_unsaved` is unchanged and the tab stays open).
    /// A close-triggered save closes the tab only after a confirmed file write
    /// ([`SaveOutcome::Saved`]) with no newer concurrent edits; a browser download
    /// (`SaveOutcome::Downloaded`) is unconfirmed and keeps the tab open and dirty.
    pub(crate) fn finish_save_copy(
        &mut self,
        ctx: &egui::Context,
        snapshot: SaveSnapshot,
        close_after: bool,
        outcome: SaveOutcome,
    ) {
        let close_confirmed = match outcome {
            SaveOutcome::Saved(dest) => {
                self.toast(ctx, format!("Saved copy to {dest}"), false);
                true
            }
            #[cfg(any(target_arch = "wasm32", test))]
            SaveOutcome::Downloaded(name) => {
                let text = if close_after {
                    format!(
                        "Started download of {name}; tab stays open until you confirm the file and close or discard"
                    )
                } else {
                    format!("Started download of {name}")
                };
                self.toast(ctx, text, false);
                false
            }
            SaveOutcome::Cancelled => false,
            SaveOutcome::Err(err) => {
                self.toast(ctx, format!("Could not save copy: {err}"), true);
                false
            }
        };
        if close_confirmed
            && close_after
            && let Some(i) = self.tabs.iter().position(|t| snapshot.matches(t))
        {
            self.close_now(i);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "unsaved_tests.rs"]
mod tests;
