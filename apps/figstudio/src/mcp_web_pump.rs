//! The relay connection between frames. A hidden tab draws no frames, and the timer
//! eframe runs it on instead is throttled to about once a minute after a few minutes
//! hidden. A dropped relay socket then stayed down until that timer came round, and every
//! call meanwhile got "No browser tab connected". The socket's own events, and timers set
//! from them, are ordinary tasks that a hidden tab still runs promptly, so they drive the
//! link here: notice a close, retry on the backoff plan, greet a new connection with
//! `ready`, start calls that waited for it, and keep it alive. Work that needs the tabs
//! themselves still waits for a frame.
use std::time::Duration;

use wasm_bindgen::{JsCast, closure::Closure};

use super::*;

/// Run [`Host::pump`] now, unless a frame (or a pump further up the stack) is using the
/// host; that one handles the same events.
fn poke(me: &Weak<RefCell<Host>>) {
    if let Some(host) = me.upgrade()
        && let Ok(mut host) = host.try_borrow_mut()
    {
        host.pump();
    }
}

impl Host {
    /// What the socket callbacks and page listeners call after queueing an event.
    pub(super) fn waker(&self) -> Rc<dyn Fn()> {
        let me = self.me.clone();
        Rc::new(move || poke(&me))
    }

    /// Between frames: apply the connection's own events, then retry, connect or ping as
    /// due. The events that need the tabs stay queued for the next frame, in order.
    fn pump(&mut self) {
        let Some(ctx) = self.ctx.clone() else {
            return;
        };
        // Every decision below schedules its own wake from this task, so the pending one
        // goes: a timer set from a socket event starts a fresh chain, which browsers do
        // not hold to a minute the way they do a long chain of timers.
        self.cancel_timer();
        let events: Vec<Event> = self.shared.events.borrow_mut().drain(..).collect();
        let mut kept = Vec::new();
        for event in events {
            let left = match event {
                Event::Rpc {
                    generation,
                    id,
                    body,
                } => self.start_call(generation, id, body),
                event => match self.link_event(event) {
                    Ok(notice) => {
                        self.notices.extend(notice);
                        None
                    }
                    Err(event) => Some(event),
                },
            };
            kept.extend(left);
        }
        {
            let mut queue = self.shared.events.borrow_mut();
            for event in kept.into_iter().rev() {
                queue.push_front(event);
            }
        }
        self.plan_retry(&ctx);
        self.connect_if_asked(&ctx);
        if self.relay
            && let Some(conn) = self.conn.as_ref().filter(|c| c.welcomed)
        {
            let ws = conn.ws.clone();
            self.beat(&ctx, &ws);
        }
        // Shows a notice, and lets a visible page catch up at once.
        ctx.request_repaint();
    }

    /// Start a relay call. Returns it when it must wait (its connection is not welcomed
    /// yet); one from a replaced socket is dropped (the relay failed it when that socket
    /// closed).
    pub(super) fn start_call(
        &mut self,
        generation: u32,
        id: String,
        body: serde_json::Value,
    ) -> Option<Event> {
        if generation != self.generation {
            return None;
        }
        let started = self
            .calls
            .borrow_mut()
            .start(generation, id.clone(), body.clone());
        (!started && self.conn.is_some()).then_some(Event::Rpc {
            generation,
            id,
            body,
        })
    }

    /// Retry a lost relay connection when its plan says so.
    pub(super) fn plan_retry(&mut self, ctx: &egui::Context) {
        if self.hurry.take() {
            self.reconnect.hurry();
        }
        if self.relay && self.conn.is_none() {
            match self.reconnect.poll(Instant::now()) {
                Retry::Now => {
                    // Quiet only when nothing else asked for this connect already.
                    self.retrying_now = !self.want_connect;
                    self.want_connect = true;
                }
                Retry::Wait(resume) => self.wake_after(ctx, resume),
                Retry::Idle => {}
            }
        }
    }

    /// Connect when asked (the menu, a link, a retry), once the cloud gate allows it.
    pub(super) fn connect_if_asked(&mut self, ctx: &egui::Context) {
        let asked = std::mem::take(&mut self.want_connect) && self.conn.is_none();
        let step = self.settle_cloud(asked);
        if step.probe {
            self.probe(ctx);
        }
        if step.connect
            && let Err(error) = self.connect()
        {
            self.notices.push(error);
        }
    }

    /// Keepalive of a welcomed relay connection.
    ///
    /// Otherwise the relay socket can sit silent for a whole session while a NAT or proxy
    /// has already forgotten the flow, and a successful send only proves the browser
    /// buffered the bytes. The keepalive state machine runs the proof: a ping goes out,
    /// the DO's runtime answers `pong` without waking it, and no answer within the
    /// deadline drops the connection (see `mcp_heartbeat.rs`). `Beat::Wait` schedules the
    /// next decision exactly, not on a fixed cadence, so the deadline lands on time (or
    /// later, in a throttled background tab; never early).
    pub(super) fn beat(&mut self, ctx: &egui::Context, ws: &WebSocket) {
        let now = Instant::now();
        match self.heartbeat.tick(now) {
            Beat::Wait(resume) => self.wake_after(ctx, resume),
            Beat::SendPing => match ws.send_with_str("ping") {
                Ok(()) => {
                    self.heartbeat.ping_sent(now);
                    self.wake_after(ctx, PONG_DEADLINE);
                }
                // A refused send is the browser already giving up on the socket:
                // disconnect now, not after some later frame notices.
                Err(_) => {
                    let msg = if self.proxy.is_some() {
                        "Local browser proxy disconnected"
                    } else {
                        "Cloud relay disconnected"
                    };
                    self.relay_lost(ctx, msg, self.heartbeat.proven())
                }
            },
            // Retried with growing delays (`relay_lost`); a connection that never
            // proved itself is one more failed attempt of the series.
            Beat::Elapsed { proven } => {
                let msg = if self.proxy.is_some() {
                    "Local browser proxy stopped responding"
                } else {
                    "Cloud relay stopped responding"
                };
                self.relay_lost(ctx, msg, proven)
            }
        }
    }

    /// Decide again after `after`: at a frame, and in a [`Self::pump`] for a hidden tab
    /// whose frames are throttled.
    pub(super) fn wake_after(&mut self, ctx: &egui::Context, after: Duration) {
        ctx.request_repaint_after(after);
        let now = Instant::now();
        let due = now + after;
        // A pending wake that is still to come and no later serves this one too.
        if self
            .timer
            .is_some_and(|(_, pending)| pending > now && pending <= due)
        {
            return;
        }
        let Some(window) = web_sys::window() else {
            return;
        };
        self.cancel_timer();
        let me = self.me.clone();
        // One callback for every wake, kept by the host: a timer never drops the closure
        // it is running.
        let tick = self.tick.get_or_insert_with(|| {
            Closure::<dyn FnMut()>::new(move || {
                if let Some(host) = me.upgrade()
                    && let Ok(mut host) = host.try_borrow_mut()
                {
                    host.timer = None;
                    host.pump();
                }
            })
        });
        let millis = i32::try_from(after.as_millis()).unwrap_or(i32::MAX);
        if let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
            tick.as_ref().unchecked_ref(),
            millis,
        ) {
            self.timer = Some((id, due));
        }
    }

    pub(super) fn cancel_timer(&mut self) {
        if let Some((id, _)) = self.timer.take()
            && let Some(window) = web_sys::window()
        {
            window.clear_timeout_with_handle(id);
        }
    }
}
