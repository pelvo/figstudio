//! Relay keepalive: prove the socket still carries frames, or drop it. Pure time logic
//! (native tests drive it with a synthetic clock); `mcp_web.rs` owns the real clock.
//!
//! A relay socket can sit silent for a whole session while a NAT or proxy has already
//! forgotten the flow, and a successful `WebSocket.send` only proves the browser
//! buffered the bytes. The only end-to-end proof is the answer to a keepalive: the
//! Worker's runtime auto-responds `ping` with `pong` without waking the Durable Object
//! (`setWebSocketAutoResponse`, web/worker.mjs). A `pong` therefore says the path
//! carried a frame to the edge and back — it does not say the DO is awake (a call wakes
//! it), which is all a keepalive needs.
//!
//! Boundaries, deliberately:
//! - a `pong` seen while a ping is outstanding clears the wait, however late it is: the
//!   host drains socket events before ticking, so a throttled frame that sees the pong
//!   first keeps the connection even past the wall-clock deadline. Direct evidence
//!   beats the deadline heuristic;
//! - a `pong` after the deadline fired changes nothing: expiry has already dropped the
//!   connection, and a new connection only trusts its own generation's `pong`;
//! - expiry drops the connection; the host retries it with growing delays
//!   (`mcp_backoff.rs`), which bounds what a black-holed path can cost. `proven` only
//!   decides whether the loss is worth a toast.
use std::time::Duration;

/// How long a proven relay connection idles between keepalive pings.
pub(crate) const PING_EVERY: Duration = Duration::from_secs(30);

/// How long a ping may stay unanswered before the connection is presumed black-holed.
/// A backgrounded tab throttles frames, so the observed deadline can only run long,
/// never short (see the module docs on late pongs).
pub(crate) const PONG_DEADLINE: Duration = Duration::from_secs(10);

/// The clock the heartbeat reads: instant differences only, so tests inject a counter.
pub(crate) trait Clock: Copy {
    fn since(self, earlier: Self) -> Duration;
}

impl Clock for web_time::Instant {
    fn since(self, earlier: Self) -> Duration {
        self.duration_since(earlier)
    }
}

/// What the host should do at this frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Beat {
    /// Nothing due; repaint again after this much time has passed.
    Wait(Duration),
    /// A keepalive ping is due now; call `ping_sent(now)` once its bytes were accepted.
    SendPing,
    /// No pong within the deadline: drop the connection. `proven` says whether this
    /// connection ever cleared a pong (an unproven one is a failed connect attempt).
    Elapsed { proven: bool },
}

/// Keepalive state for one relay connection.
pub(crate) struct Heartbeat<T: Clock> {
    /// Only a pong tagged with this generation clears a wait: one drained from a socket
    /// the host has already replaced must not speak for the new connection.
    generation: u32,
    state: State<T>,
}

/// The three situations a relay connection is in.
enum State<T: Clock> {
    /// Nothing outstanding. `last_pong` is `None` until this connection's first pong.
    Idle { last_pong: Option<T> },
    /// A ping is outstanding since `sent`. `proven`: a pong was cleared once.
    AwaitingPong { sent: T, proven: bool },
    /// The deadline fired; the connection it belonged to is gone.
    Elapsed { proven: bool },
}

impl<T: Clock> Heartbeat<T> {
    /// A connection starting: nothing proven, first ping due right away.
    pub(crate) fn fresh(generation: u32) -> Self {
        Self {
            generation,
            state: State::Idle { last_pong: None },
        }
    }

    /// The frame's decision; passing the deadline moves the machine to `Elapsed`, so
    /// a pong drained later cannot clear what the host already gave up on. A late
    /// frame (throttled tab) re-decides by the same rule.
    pub(crate) fn tick(&mut self, now: T) -> Beat {
        match self.state {
            // Fresh connection: no pong yet, so a ping is due right away — it validates
            // the path immediately instead of after a silent interval.
            State::Idle { last_pong: None } => Beat::SendPing,
            State::Idle {
                last_pong: Some(pong),
            } => {
                let since = now.since(pong);
                if since >= PING_EVERY {
                    Beat::SendPing
                } else {
                    Beat::Wait(PING_EVERY - since)
                }
            }
            State::AwaitingPong { sent, proven } => {
                let since = now.since(sent);
                if since >= PONG_DEADLINE {
                    self.state = State::Elapsed { proven };
                    Beat::Elapsed { proven }
                } else {
                    Beat::Wait(PONG_DEADLINE - since)
                }
            }
            State::Elapsed { proven } => Beat::Elapsed { proven },
        }
    }

    /// The bytes left the browser; the wait for this ping starts now.
    pub(crate) fn ping_sent(&mut self, now: T) {
        let proven = self.proven();
        self.state = State::AwaitingPong { sent: now, proven };
    }

    /// The relay answered. A foreign generation (a socket the host already replaced)
    /// and an expired connection are both ignored: a pong can never revive anything.
    pub(crate) fn pong(&mut self, generation: u32, now: T) {
        if generation != self.generation || matches!(self.state, State::Elapsed { .. }) {
            return;
        }
        self.state = State::Idle {
            last_pong: Some(now),
        };
    }

    /// Whether this connection ever cleared a pong.
    pub(crate) fn proven(&self) -> bool {
        match self.state {
            State::Idle { last_pong: Some(_) } => true,
            State::Idle { last_pong: None } => false,
            State::AwaitingPong { proven, .. } | State::Elapsed { proven } => proven,
        }
    }
}

#[cfg(test)]
#[path = "mcp_heartbeat_tests.rs"]
mod tests;
