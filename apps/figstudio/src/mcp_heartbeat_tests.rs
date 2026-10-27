//! Relay keepalive state machine tests: a synthetic millisecond clock drives every
//! boundary — cadence, deadline, late pongs, stale generations, whether a connection proved itself.
use super::*;
use std::time::Duration;

/// Synthetic monotonic clock: milliseconds since an arbitrary epoch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Ticks(u64);

impl Clock for Ticks {
    fn since(self, earlier: Self) -> Duration {
        Duration::from_millis(self.0.saturating_sub(earlier.0))
    }
}

const T0: Ticks = Ticks(0);
const S: u64 = 1_000;

fn at(seconds: u64) -> Ticks {
    Ticks(seconds * S)
}

fn assert_wait(beat: Beat, expected: Duration) {
    match beat {
        Beat::Wait(resume) => assert_eq!(resume, expected, "expected to wait {expected:?}"),
        other => panic!("expected Wait({expected:?}), got {other:?}"),
    }
}

fn assert_send_ping(beat: Beat) {
    assert!(
        matches!(beat, Beat::SendPing),
        "expected SendPing, got {beat:?}"
    );
}

fn assert_elapsed(beat: Beat, proven: bool) {
    match beat {
        Beat::Elapsed { proven: got } => {
            assert_eq!(got, proven, "expected Elapsed {{ proven: {proven} }}")
        }
        other => panic!("expected Elapsed, got {other:?}"),
    }
}

/// A connection whose ping left `sent` seconds ago, still awaiting its pong.
fn awaiting(generation: u32, sent: u64) -> Heartbeat<Ticks> {
    let mut heartbeat = Heartbeat::fresh(generation);
    heartbeat.ping_sent(at(sent));
    heartbeat
}

#[test]
fn a_fresh_connection_pings_immediately_then_idles_until_answered() {
    // `None` right after connecting: the first ping validates the socket right away.
    assert_send_ping(Heartbeat::<Ticks>::fresh(1).tick(T0));
    // While the answer is outstanding the deadline is the only clock that matters.
    let mut waiting = awaiting(1, 0);
    assert_wait(waiting.tick(T0), PONG_DEADLINE);
    assert_wait(waiting.tick(at(9)), Duration::from_secs(1));
}

#[test]
fn an_answered_ping_idles_for_the_full_interval_since_the_pong() {
    let mut answered = awaiting(1, 0);
    answered.pong(1, at(2));
    // The next ping is due PING_EVERY after the *pong*, not after the ping.
    assert_wait(answered.tick(at(2 + 29)), Duration::from_secs(1));
    assert_send_ping(answered.tick(at(2 + 30)));
}

#[test]
fn a_missing_pong_expires_exactly_at_the_deadline() {
    let mut waiting = awaiting(1, 0);
    assert_wait(waiting.tick(at(9)), PONG_DEADLINE - Duration::from_secs(9));
    assert_elapsed(waiting.tick(at(10)), false);
    assert_elapsed(waiting.tick(at(60)), false);
}

#[test]
fn an_expired_connection_reports_whether_it_was_ever_proven() {
    // A connection that cleared one pong is proven; its later expiry is worth a toast.
    let mut proven = awaiting(1, 0);
    proven.pong(1, at(2));
    assert_send_ping(proven.tick(at(32)));
    let mut again = awaiting(1, 0);
    again.pong(1, at(2));
    again.ping_sent(at(32));
    assert_elapsed(again.tick(at(42)), true);
    // The deadline holds for the very first ping too (never proven, never retried).
    assert_elapsed(awaiting(1, 0).tick(at(10)), false);
}

#[test]
fn expiry_is_stable_and_a_late_pong_cannot_revive_it() {
    let mut expired = awaiting(1, 0);
    assert_elapsed(expired.tick(at(10)), false);
    // A pong arriving after the host already saw the deadline changes nothing.
    expired.pong(1, at(11));
    assert_elapsed(expired.tick(at(11)), false);
}

#[test]
fn a_pong_seen_while_awaiting_counts_however_late_it_is() {
    // Throttled-frame boundary: the host drains socket events before ticking, so a
    // pong that arrives past the wall-clock deadline still wins if no tick saw the
    // deadline first. The machine never rejects a pong for being late — only the
    // host's ordering decides.
    let mut revived = awaiting(1, 0);
    revived.pong(1, at(10 + 5));
    assert_wait(revived.tick(at(10 + 5)), PING_EVERY);
}

#[test]
fn a_pong_from_a_stale_generation_never_clears_the_new_connection_s_wait() {
    // The host bumped the generation (new socket); the old socket's drained pong
    // must not answer the new connection's ping.
    let mut ignored = awaiting(2, 0);
    ignored.pong(1, at(1));
    assert_elapsed(ignored.tick(at(10)), false);
    // The new connection's own pong still clears it.
    let mut answered = awaiting(2, 0);
    answered.pong(2, at(1));
    assert_wait(answered.tick(at(1)), PING_EVERY);
}

#[test]
fn a_reset_heartbeat_starts_over_with_the_new_generation() {
    // The old machine was dropped wholesale in practice; a fresh one pings again.
    let mut fresh = Heartbeat::<Ticks>::fresh(3);
    assert_send_ping(fresh.tick(T0));
    fresh.ping_sent(T0);
    fresh.pong(3, at(1));
    assert_wait(fresh.tick(at(1)), PING_EVERY);
}
