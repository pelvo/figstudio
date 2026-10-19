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
/// A start this soon after the fingers lifted is the trackpad's momentum, not new fingers
/// (nobody lifts and touches again that fast).
const MOMENTUM: f64 = 0.025;
/// A start in the frame right after the lift is momentum too, however long that frame took
/// (the UI thread hitched), up to this long.
const MOMENTUM_FRAME: f64 = 0.2;
/// A wheel without phases (a plain mouse wheel) has ended after this pause.
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
    /// When (time and frame) the fingers last lifted (a phased End), until its momentum.
    lifted: Option<(f64, u64)>,
    /// Inside the trackpad's momentum after a lift: ignored up to its own End. New fingers
    /// end it (macOS ends the momentum, then starts the new gesture).
    momentum: bool,
    last: f64,
}

impl Swipe {
    /// Feeds one frame's scroll events, in order (`delta` in points, content direction), at
    /// time `now` in frame number `frame`. The events of a frame share its time, so moves in
    /// a row count as one: their sum over the time since the last frame is the speed (a
    /// trackpad sends several a frame). Returns how a horizontal swipe ended.
    pub fn feed_frame(
        &mut self,
        events: &[(Vec2, TouchPhase)],
        now: f64,
        frame: u64,
    ) -> Option<End> {
        let mut ended = None;
        let mut moved: Option<Vec2> = None;
        for &(delta, phase) in events {
            if phase == TouchPhase::Move {
                *moved.get_or_insert(Vec2::ZERO) += delta;
                continue;
            }
            if let Some(sum) = moved.take() {
                ended = ended.or(self.event(sum, TouchPhase::Move, now, frame));
            }
            ended = ended.or(self.event(delta, phase, now, frame));
        }
        if let Some(sum) = moved {
            ended = ended.or(self.event(sum, TouchPhase::Move, now, frame));
        }
        ended
    }

    /// One scroll event in its own frame (a frame number far from any other).
    #[cfg(test)]
    fn feed(&mut self, delta: Vec2, phase: TouchPhase, now: f64) -> Option<End> {
        self.feed_frame(&[(delta, phase)], now, (now * 1000.0) as u64 * 10)
    }

    fn event(&mut self, delta: Vec2, phase: TouchPhase, now: f64, frame: u64) -> Option<End> {
        let gap = now - self.last;
        self.last = now;
        let momentum = self.lifted.is_some_and(|(t, f)| {
            now - t <= MOMENTUM || (frame <= f + 1 && now - t <= MOMENTUM_FRAME)
        });
        match phase {
            TouchPhase::Start if momentum => {
                self.momentum = true;
                None
            }
            TouchPhase::Start => {
                self.restart(true);
                None
            }
            TouchPhase::End | TouchPhase::Cancel if self.momentum => {
                // One momentum per lift: a start after it is new fingers.
                self.momentum = false;
                self.lifted = None;
                None
            }
            TouchPhase::End | TouchPhase::Cancel => {
                self.phased = true;
                self.lifted = Some((now, frame));
                self.finish()
            }
            TouchPhase::Move if self.momentum => None,
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
            lifted: self.lifted,
            ..Self::default()
        };
    }

    fn finish(&mut self) -> Option<End> {
        let end = self.offset().map(|offset| End {
            offset,
            velocity: self.velocity,
        });
        *self = Self {
            last: self.last,
            lifted: self.lifted,
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
        // The momentum right after the lift (macOS starts it as a new scroll) is no swipe.
        s.feed(Vec2::ZERO, TouchPhase::Start, 0.072);
        for i in 1..10 {
            s.feed(
                vec2(-20.0, 0.0),
                TouchPhase::Move,
                0.072 + f64::from(i) * 0.016,
            );
            assert_eq!(s.offset(), None, "momentum {i}");
        }
        // New fingers stop the momentum and swipe again at once, no pause needed.
        assert_eq!(
            s.feed(Vec2::ZERO, TouchPhase::End, 0.25),
            None,
            "the momentum ends"
        );
        s.feed(Vec2::ZERO, TouchPhase::Start, 0.26);
        s.feed(vec2(30.0, 0.0), TouchPhase::Move, 0.276);
        assert_eq!(s.offset(), Some(30.0));
        assert!(s.feed(Vec2::ZERO, TouchPhase::End, 0.29).is_some());
        // A slow lift has no momentum: the next touch swipes.
        s.feed(Vec2::ZERO, TouchPhase::Start, 0.5);
        s.feed(vec2(-30.0, 0.0), TouchPhase::Move, 0.516);
        assert_eq!(s.offset(), Some(-30.0));
    }

    #[test]
    fn a_flick_whose_moves_share_a_frame_is_still_quick() {
        let mut s = Swipe::default();
        s.feed_frame(&[(Vec2::ZERO, TouchPhase::Start)], 0.0, 1);
        s.feed_frame(&[(vec2(-12.0, 0.0), TouchPhase::Move)], 0.016, 2);
        // A trackpad sends several moves in one frame; they all have the frame's time.
        let burst = [(vec2(-10.0, 0.0), TouchPhase::Move); 4];
        s.feed_frame(&burst, 0.032, 3);
        let end = s
            .feed_frame(&[(Vec2::ZERO, TouchPhase::End)], 0.048, 4)
            .expect("ended");
        assert_eq!(end.offset, -52.0);
        assert!(end.velocity <= -FLICK_SPEED, "{}", end.velocity);
        assert_eq!(end.turn(200.0), -1, "a flick, not a slow drag");
    }

    #[test]
    fn momentum_after_a_hitch_and_new_fingers_after_momentum_are_told_apart() {
        let swipe = |s: &mut Swipe, t: f64, frame: u64| {
            s.feed_frame(&[(Vec2::ZERO, TouchPhase::Start)], t, frame);
            s.feed_frame(
                &[(vec2(-40.0, 0.0), TouchPhase::Move)],
                t + 0.016,
                frame + 1,
            );
            s.feed_frame(&[(Vec2::ZERO, TouchPhase::End)], t + 0.032, frame + 2)
        };
        let mut s = Swipe::default();
        assert!(swipe(&mut s, 0.0, 1).is_some());
        // The UI thread hitched for 80 ms: the momentum's start comes in the next frame.
        assert_eq!(
            swipe(&mut s, 0.112, 4),
            None,
            "momentum, not a second swipe"
        );
        // A start right after that momentum ended is new fingers, not more momentum.
        let mut s = Swipe::default();
        assert!(swipe(&mut s, 0.0, 1).is_some());
        s.feed_frame(&[(Vec2::ZERO, TouchPhase::Start)], 0.036, 4);
        s.feed_frame(&[(Vec2::ZERO, TouchPhase::End)], 0.040, 5);
        assert!(swipe(&mut s, 0.050, 20).is_some(), "a new swipe");
        // Long after a lift, an idle app's next frame is a new swipe too.
        assert!(swipe(&mut s, 2.0, 23).is_some());
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
