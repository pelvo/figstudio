//! Reconnect pacing tests: a synthetic second clock drives the delays, their cap, the
//! reset after a proven connection and the early retry on visible or online.
use super::*;

/// Synthetic monotonic clock: seconds since an arbitrary epoch.
#[derive(Clone, Copy, Debug)]
struct Secs(u64);

impl Clock for Secs {
    fn since(self, earlier: Self) -> Duration {
        Duration::from_secs(self.0.saturating_sub(earlier.0))
    }
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

#[test]
fn nothing_is_planned_until_a_loss() {
    let mut reconnect = Reconnect::default();
    assert_eq!(reconnect.poll(Secs(0)), Retry::Idle);
    assert!(!reconnect.retrying());
}

#[test]
fn a_lost_connection_is_retried_after_the_first_delay() {
    let mut reconnect = Reconnect::default();
    reconnect.lost(Secs(10));
    assert_eq!(reconnect.poll(Secs(10)), Retry::Wait(FIRST_RETRY));
    assert_eq!(reconnect.poll(Secs(11)), Retry::Now);
    // The plan is used up; only another loss plans the next retry.
    assert_eq!(reconnect.poll(Secs(12)), Retry::Idle);
}

#[test]
fn failed_retries_double_the_delay_up_to_the_cap() {
    let mut reconnect = Reconnect::default();
    let mut now = 0;
    let mut waits = Vec::new();
    for _ in 0..9 {
        reconnect.lost(Secs(now));
        let Retry::Wait(wait) = reconnect.poll(Secs(now)) else {
            panic!("a loss must plan a retry");
        };
        waits.push(wait.as_secs());
        now += wait.as_secs();
        assert_eq!(reconnect.poll(Secs(now)), Retry::Now);
    }
    assert_eq!(waits, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    assert!(reconnect.retrying());
}

#[test]
fn a_proven_connection_starts_the_next_series_from_the_first_delay() {
    let mut reconnect = Reconnect::default();
    for now in [0, 1, 3] {
        reconnect.lost(Secs(now));
        reconnect.poll(Secs(now + 10));
    }
    reconnect.proven();
    assert!(!reconnect.retrying());
    reconnect.lost(Secs(100));
    assert_eq!(reconnect.poll(Secs(100)), Retry::Wait(FIRST_RETRY));
}

#[test]
fn becoming_visible_or_online_retries_at_once() {
    let mut reconnect = Reconnect::default();
    for _ in 0..6 {
        reconnect.lost(Secs(0));
    }
    assert_eq!(reconnect.poll(Secs(5)), Retry::Wait(secs(27)));
    reconnect.hurry();
    assert_eq!(reconnect.poll(Secs(5)), Retry::Now);
    // With nothing planned, hurrying plans nothing.
    reconnect.hurry();
    assert_eq!(reconnect.poll(Secs(5)), Retry::Idle);
}

#[test]
fn a_cancelled_plan_never_fires() {
    let mut reconnect = Reconnect::default();
    reconnect.lost(Secs(0));
    reconnect.cancel();
    assert_eq!(reconnect.poll(Secs(1_000)), Retry::Idle);
}
