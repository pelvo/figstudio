//! Relay calls between frames: the socket callback starts them, a worker wake sends
//! their replies. A hidden tab draws no frames, and the timer eframe runs it on instead
//! can be throttled to once a minute, longer than the relay's 30 s call deadline.
use super::*;
use crate::mcp_relay::{Done, Inbox, RelayTabs};

#[derive(Debug, Default)]
pub(super) struct Calls {
    pub inbox: Inbox,
    /// The welcomed relay socket and its connection generation; `None` while there is
    /// none. Set by each frame (`McpHost::publish`).
    live: Option<(u32, WebSocket)>,
    /// The tabs as the last frame saw them; calls between frames run against these.
    view: RelayTabs,
}

impl Calls {
    /// What the last frame saw. Captured only while a relay socket is live.
    pub fn publish(&mut self, live: Option<(u32, WebSocket)>, tabs: &[Tab], active: usize) {
        self.view = if live.is_some() {
            RelayTabs::capture(tabs, active)
        } else {
            RelayTabs::default()
        };
        self.live = live;
    }

    /// Start a call that arrived on connection `generation`. `false` when that connection
    /// is not the live one (yet): the caller queues it for the frame.
    pub fn start(&mut self, generation: u32, id: String, body: serde_json::Value) -> bool {
        let Some((live, ws)) = &self.live else {
            return false;
        };
        if *live != generation {
            return false;
        }
        // A refused call is answered immediately (the worker would otherwise hold it for
        // the full 30 s); an accepted one replies from the pool.
        if let Some(busy) = self.inbox.dispatch_view(generation, id, body, &self.view) {
            let _ = ws.send_with_str(&busy);
        }
        true
    }

    /// Send the finished calls' replies on the connection they came from; a reply for a
    /// replaced connection is dropped (its caller was failed when that socket closed).
    pub fn flush(&mut self) {
        for (generation, text) in self.inbox.take_replies() {
            if let Some((live, ws)) = &self.live
                && *live == generation
            {
                let _ = ws.send_with_str(&text);
            }
        }
    }

    /// Reveals for the UI frame to apply; [`Self::flush`] first.
    pub fn take_reveals(&mut self) -> std::collections::VecDeque<Done> {
        self.inbox.take_reveals()
    }
}
