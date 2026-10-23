//! Relay reconnect pacing: a lost relay connection is retried with growing delays.
//! Pure time logic (native tests drive it with a synthetic clock); `mcp_web.rs` owns the
//! real clock and the socket.
//!
//! Before this, a relay socket that closed (a Worker deploy, a network change, a sleeping
//! laptop) stayed closed until someone reconnected by hand, so every call in the meantime
//! got "No browser tab connected". The delays double from [`FIRST_RETRY`] to
//! [`MAX_RETRY`], which bounds the load a black-holed path can cause (the old one-retry
//! storm bound); a connection that proves itself (a keepalive `pong`) starts the next
//! series from the first delay again. Each wait adds up to [`JITTER`] of its delay at
//! random, so the tabs one Worker deploy dropped together do not all retry together.
//! Becoming visible or coming back online retries at once instead of waiting out the
//! delay.
use std::time::Duration;

use crate::mcp_heartbeat::Clock;

/// The first retry's delay after a loss.
pub(crate) const FIRST_RETRY: Duration = Duration::from_secs(1);

/// The longest delay between two retries (before [`JITTER`]).
pub(crate) const MAX_RETRY: Duration = Duration::from_secs(60);

/// The most a wait adds to its delay, as a share of it.
pub(crate) const JITTER: f64 = 0.25;

/// What the host should do at this frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Retry {
    /// No retry is planned.
    Idle,
    /// A retry is planned; repaint again after this much time has passed.
    Wait(Duration),
    /// Connect now. The plan is used up: a failed attempt calls `lost` again.
    Now,
}

/// Reconnect plan for the relay connection.
#[derive(Debug)]
pub(crate) struct Reconnect<T: Clock> {
    /// The planned retry: when the loss was seen, and how long to wait from then.
    planned: Option<(T, Duration)>,
    /// The delay the next loss waits.
    next: Duration,
}

impl<T: Clock> Default for Reconnect<T> {
    fn default() -> Self {
        Self {
            planned: None,
            next: FIRST_RETRY,
        }
    }
}

impl<T: Clock> Reconnect<T> {
    /// The connection (or a connect attempt) was lost at `now`: plan a retry after the
    /// current delay plus `spread` (a random number in `0..1`, clamped) times
    /// [`JITTER`] of it, and double the delay for the loss after that.
    pub(crate) fn lost(&mut self, now: T, spread: f64) {
        let extra = self.next.mul_f64(spread.clamp(0.0, 1.0) * JITTER);
        self.planned = Some((now, self.next + extra));
        self.next = (self.next * 2).min(MAX_RETRY);
    }

    /// The connection answered a keepalive: the next loss retries quickly again.
    pub(crate) fn proven(&mut self) {
        self.next = FIRST_RETRY;
    }

    /// Someone disconnected by hand: drop the planned retry.
    pub(crate) fn cancel(&mut self) {
        self.planned = None;
    }

    /// A connect is starting. A planned retry (`planned`) keeps the series, so its delay
    /// keeps growing and its failure stays quiet. Any other connect (by hand, a new link)
    /// starts afresh, so its failure is reported.
    pub(crate) fn starting(&mut self, planned: bool) {
        if planned {
            self.cancel();
        } else {
            *self = Self::default();
        }
    }

    /// The page became visible or the browser came back online: a planned retry is due
    /// now instead of after its delay.
    pub(crate) fn hurry(&mut self) {
        if let Some((_, wait)) = &mut self.planned {
            *wait = Duration::ZERO;
        }
    }

    /// Whether losses since the last proven connection are being retried; their failed
    /// attempts need no toast each.
    pub(crate) fn retrying(&self) -> bool {
        self.next > FIRST_RETRY
    }

    /// The frame's decision. `Now` clears the plan.
    pub(crate) fn poll(&mut self, now: T) -> Retry {
        let Some((since, wait)) = self.planned else {
            return Retry::Idle;
        };
        let waited = now.since(since);
        if waited >= wait {
            self.planned = None;
            Retry::Now
        } else {
            Retry::Wait(wait - waited)
        }
    }
}

#[cfg(test)]
#[path = "mcp_backoff_tests.rs"]
mod tests;
