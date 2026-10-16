//! The browser wiring for linked-room joins: watch the fragment, ask, then adopt.
use super::*;
use crate::mcp_join::{JoinReply, join_dialog};
use wasm_bindgen::{JsCast, closure::Closure};

/// `hashchange` cannot wake the UI thread; `sync` reads this cell every frame.
pub(super) fn watch_fragment(link: Rc<RefCell<Option<Link>>>) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let (link_cell, target) = (link, window.clone());
    let on_hash = Closure::<dyn FnMut()>::new(move || {
        let hash = target.location().hash().ok();
        *link_cell.borrow_mut() = Some(parse_link(hash.as_deref()));
    });
    let _ = window.add_event_listener_with_callback("hashchange", on_hash.as_ref().unchecked_ref());
    // The listener lives as long as the page does.
    on_hash.forget();
}

impl McpHost {
    /// Ask about a pending linked room before anything joins it. Drawn ahead of the app
    /// shortcuts so the dialog's Escape is consumed first.
    pub fn confirm(&mut self, ctx: &egui::Context) {
        let Some(proposal) = self.proposal.clone() else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let mut reply = None;
        egui::Window::new("Join another browser's room?")
            .fixed_pos((40.0, 96.0))
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| reply = join_dialog(ui, &proposal));
        match reply {
            Some(JoinReply::Join) => {
                // The same contract as `Apply` in the menu: a room is only adopted once
                // its executor could be saved.
                match room::save(proposal.pair()) {
                    Ok(()) => self.adopt(proposal.into_pair()),
                    Err(error) => self.notices.push(error),
                }
                self.proposal = None;
            }
            // Declining changes nothing: room, storage and connection stay as they are.
            Some(JoinReply::Stay) => self.proposal = None,
            None => {}
        }
    }

    /// Make `pair` this tab's credential and reconnect the relay to its room.
    pub(super) fn adopt(&mut self, pair: ExecutorPair) {
        self.stop();
        self.pair = Some(pair);
        self.relay = true;
        self.want_connect = true;
    }
}
