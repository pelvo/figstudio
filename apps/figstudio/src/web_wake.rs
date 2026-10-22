//! Wake the page's main thread from a pool worker without waiting for a frame.
//!
//! A hidden tab gets no animation frames: eframe falls back to a timer that browsers
//! clamp to about once a second, and Chrome's intensive throttling to about once a minute
//! after a few minutes hidden. Relay replies and image reads that waited for the next
//! frame could then outlive the relay's 30 s call deadline. A `BroadcastChannel` message
//! is an ordinary task, not a timer, so it runs promptly in a hidden tab.
//!
//! Workers post on a channel named for this page load; the main thread listens and runs
//! the registered pumps. Wakes coalesce: at most one message is in flight.
use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{BroadcastChannel, MessageEvent};

/// The channel name; set once the first [`listen`] is receiving, so a wake before that is
/// a no-op (a message posted with nobody listening would leave [`POSTED`] stuck).
static NAME: OnceLock<String> = OnceLock::new();
/// A wake message is in flight; cleared by the listener before it runs the pumps.
static POSTED: AtomicBool = AtomicBool::new(false);

type Listener = (BroadcastChannel, Closure<dyn FnMut(MessageEvent)>);

thread_local! {
    /// This thread's sending end. Each worker opens its own: JS objects stay on the
    /// thread that made them.
    static SENDER: RefCell<Option<BroadcastChannel>> = const { RefCell::new(None) };
    /// Main thread: what a wake runs.
    static PUMPS: RefCell<Vec<Box<dyn FnMut()>>> = RefCell::new(Vec::new());
    /// Main thread: the receiving end, alive as long as the page.
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

/// Run `pump` on the main thread after each [`wake`]. Call on the main thread.
pub fn listen(pump: impl FnMut() + 'static) {
    PUMPS.with_borrow_mut(|pumps| pumps.push(Box::new(pump)));
    LISTENER.with_borrow_mut(|listener| {
        if listener.is_some() {
            return;
        }
        // Per page load, so another tab of this origin never runs this page's pumps.
        let salt = (js_sys::Math::random() * 9_007_199_254_740_992.0) as u64;
        let name = format!("figstudio-wake-{salt:016x}");
        let Ok(channel) = BroadcastChannel::new(&name) else {
            return;
        };
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(|_| run());
        channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        *listener = Some((channel, on_message));
        POSTED.store(false, Ordering::Release);
        let _ = NAME.set(name);
    });
}

/// Ask the main thread to run the pumps soon. Any thread may call it.
pub fn wake() {
    let Some(name) = NAME.get() else {
        return;
    };
    if POSTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let sent = SENDER.with_borrow_mut(|sender| {
        if sender.is_none() {
            *sender = BroadcastChannel::new(name).ok();
        }
        sender
            .as_ref()
            .is_some_and(|channel| channel.post_message(&JsValue::NULL).is_ok())
    });
    if !sent {
        // Nothing is in flight, so the next wake must try again.
        POSTED.store(false, Ordering::Release);
    }
}

fn run() {
    // Cleared first: work queued while the pumps run posts a fresh wake.
    POSTED.store(false, Ordering::Release);
    // Taken out while running, so a pump that registers another does not double-borrow.
    let mut pumps = PUMPS.with_borrow_mut(std::mem::take);
    for pump in &mut pumps {
        pump();
    }
    PUMPS.with_borrow_mut(|added| {
        pumps.append(added);
        *added = pumps;
    });
}
