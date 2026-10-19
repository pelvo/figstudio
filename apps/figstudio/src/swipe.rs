//! Two-finger horizontal swipes over the sidebar move between spaces, as in Arc: the two
//! spaces sit side by side and follow the fingers; letting go past a third of the sidebar,
//! or with a quick flick, turns the page.

use egui::{TouchPhase, Vec2};

/// How far (points) the fingers must travel before the swipe picks an axis.
const AXIS_LOCK: f32 = 10.0;
/// Share of the sidebar's width a slow swipe must cover to turn the page.
pub const TURN_SHARE: f32 = 0.33;
/// A flick this fast (points per second) turns the page from [`FLICK_MIN`] points on.
pub const FLICK_SPEED: f32 = 900.0;
pub const FLICK_MIN: f32 = 40.0;
/// Events closer together than this belong to the same stream: after the fingers lift, the
/// trackpad's momentum (even when it reports a new start) is ignored until such a pause.
const PAUSE: f64 = 0.12;

/// How a swipe ended: how far the content moved (positive: right) and how fast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct End {
    pub offset: f32,
    pub velocity: f32,
}

impl End {
    /// `-1` for the next space, `1` for the previous one, `0` to stay, for a sidebar `width`
    /// points wide.
    pub fn turn(self, width: f32) -> i32 {
        let far = self.offset.abs() >= width * TURN_SHARE;
        let flick = self.velocity.abs() >= FLICK_SPEED
            && self.offset.abs() >= FLICK_MIN
            && self.velocity.signum() == self.offset.signum();
        if far || flick {
            if self.offset < 0.0 { -1 } else { 1 }
        } else {
            0
        }
    }
}

/// One swipe in progress.
#[derive(Debug, Default)]
pub struct Swipe {
    active: bool,
    /// `Some(true)`: horizontal (a swipe); `Some(false)`: vertical (a scroll, ignored).
    axis: Option<bool>,
    /// Movement before the axis is known.
    pending: Vec2,
    /// How far the content moved (positive: right), once horizontal.
    offset: f32,
    /// Smoothed speed of the content, points per second.
    velocity: f32,
    /// The device reports phases (a trackpad): its End finishes the swipe.
    phased: bool,
    /// Ignoring the stream after a swipe ended (momentum), until a pause.
    coasting: bool,
    last: f64,
}

impl Swipe {
    /// Feeds one scroll event (`delta` in points, content direction). Returns how a
    /// horizontal swipe ended.
    pub fn feed(&mut self, delta: Vec2, phase: TouchPhase, now: f64) -> Option<End> {
        let gap = now - self.last;
        self.last = now;
        if self.coasting {
            if gap < PAUSE {
                return None;
            }
            self.coasting = false;
        }
        match phase {
            TouchPhase::Start => {
                self.restart(true);
                None
            }
            TouchPhase::End | TouchPhase::Cancel => {
                self.phased = true;
                self.finish()
            }
            TouchPhase::Move => {
                if !self.active {
                    // A device without phases: the first event starts the swipe.
                    self.restart(false);
                }
                if gap > 0.0 && gap < 0.1 {
                    let speed = delta.x / gap as f32;
                    self.velocity = self.velocity * 0.6 + speed * 0.4;
                }
                match self.axis {
                    None => {
                        self.pending += delta;
                        if self.pending.x.abs().max(self.pending.y.abs()) > AXIS_LOCK {
                            let horizontal = self.pending.x.abs() > self.pending.y.abs() * 1.5;
                            self.axis = Some(horizontal);
                            if horizontal {
                                self.offset = self.pending.x;
                            }
                        }
                    }
                    Some(true) => self.offset += delta.x,
                    Some(false) => {}
                }
                None
            }
        }
    }

    /// Ends a swipe from a device without phases once it pauses.
    pub fn tick(&mut self, now: f64) -> Option<End> {
        if self.active && !self.phased && now - self.last > PAUSE * 2.0 {
            return self.finish();
        }
        None
    }

    /// Whether a swipe (or a scroll that may become one) is going on.
    pub fn is_on(&self) -> bool {
        self.active
    }

    /// The content's offset while a horizontal swipe is on.
    pub fn offset(&self) -> Option<f32> {
        (self.active && self.axis == Some(true)).then_some(self.offset)
    }

    fn restart(&mut self, phased: bool) {
        *self = Self {
            active: true,
            phased,
            last: self.last,
            ..Self::default()
        };
    }

    fn finish(&mut self) -> Option<End> {
        let end = self.offset().map(|offset| End {
            offset,
            velocity: self.velocity,
        });
        let last = self.last;
        *self = Self {
            // What follows a finished swipe right away is its momentum.
            coasting: end.is_some() || self.phased,
            last,
            ..Self::default()
        };
        end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn a_trackpad_swipe_follows_the_fingers_and_ignores_its_momentum() {
        let mut s = Swipe::default();
        assert_eq!(s.feed(Vec2::ZERO, TouchPhase::Start, 0.0), None);
        s.feed(vec2(-6.0, 1.0), TouchPhase::Move, 0.016);
        assert_eq!(s.offset(), None, "no axis yet");
        s.feed(vec2(-10.0, 1.0), TouchPhase::Move, 0.032);
        assert_eq!(s.offset(), Some(-16.0));
        s.feed(vec2(-60.0, 3.0), TouchPhase::Move, 0.048);
        let end = s.feed(Vec2::ZERO, TouchPhase::End, 0.064).expect("ended");
        assert_eq!(end.offset, -76.0);
        assert!(end.velocity < -500.0, "{}", end.velocity);
        // Momentum, even one that reports a new start, is not another swipe.
        s.feed(Vec2::ZERO, TouchPhase::Start, 0.07);
        for i in 1..10 {
            s.feed(
                vec2(-20.0, 0.0),
                TouchPhase::Move,
                0.07 + f64::from(i) * 0.016,
            );
            assert_eq!(s.offset(), None, "momentum {i}");
        }
        // After a pause a new swipe starts.
        s.feed(Vec2::ZERO, TouchPhase::Start, 1.0);
        s.feed(vec2(30.0, 0.0), TouchPhase::Move, 1.016);
        assert_eq!(s.offset(), Some(30.0));
    }

    #[test]
    fn a_slow_short_swipe_stays_and_a_long_or_quick_one_turns() {
        let width = 240.0;
        let stay = End {
            offset: -60.0,
            velocity: -200.0,
        };
        assert_eq!(stay.turn(width), 0, "too little force");
        let far = End {
            offset: -90.0,
            velocity: -100.0,
        };
        assert_eq!(far.turn(width), -1, "past a third");
        let flick = End {
            offset: 50.0,
            velocity: 1500.0,
        };
        assert_eq!(flick.turn(width), 1, "a quick flick");
        let back = End {
            offset: -90.0,
            velocity: 1500.0,
        };
        assert_eq!(
            back.turn(width),
            -1,
            "a flick back does not reverse a long swipe"
        );
        let tiny = End {
            offset: -20.0,
            velocity: -3000.0,
        };
        assert_eq!(tiny.turn(width), 0, "a twitch is not a flick");
    }

    #[test]
    fn a_vertical_scroll_is_not_a_swipe() {
        let mut s = Swipe::default();
        s.feed(Vec2::ZERO, TouchPhase::Start, 0.0);
        s.feed(vec2(3.0, -20.0), TouchPhase::Move, 0.01);
        s.feed(vec2(40.0, -20.0), TouchPhase::Move, 0.02);
        assert_eq!(s.offset(), None);
        assert_eq!(s.feed(Vec2::ZERO, TouchPhase::End, 0.03), None);
    }

    #[test]
    fn a_wheel_without_phases_ends_after_a_pause() {
        let mut s = Swipe::default();
        s.feed(vec2(30.0, 0.0), TouchPhase::Move, 0.0);
        s.feed(vec2(40.0, 0.0), TouchPhase::Move, 0.05);
        assert_eq!(s.offset(), Some(70.0));
        assert_eq!(s.tick(0.2), None, "still going");
        assert_eq!(s.tick(0.5).map(|e| e.offset), Some(70.0));
        assert_eq!(s.offset(), None);
    }
}
