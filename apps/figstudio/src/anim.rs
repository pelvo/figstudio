//! Motion: springs for things that move or pop, short eased fades for colour.
//!
//! State lives in egui's temporary memory under the caller's [`Id`], advances with egui's
//! clock (so it does not depend on the frame rate), and asks for repaints only while it moves.
//! With motion reduced (View > Reduce motion, or a host that turned egui's animations off,
//! such as the test harness) every value jumps straight to its target.

use egui::{Color32, Context, Id, Rect};

/// egui's own animations (panel slides, popups, collapsing): a little slower than its default.
pub const EGUI_ANIMATION_TIME: f32 = 0.18;

/// How a value springs toward its target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    /// Quick, settles without a visible bounce: highlights sliding between rows.
    pub const SMOOTH: Spring = Spring {
        stiffness: 420.0,
        damping: 38.0,
    };
    /// A small overshoot: pop-ins (command bar, toasts) and presses.
    pub const BOUNCY: Spring = Spring {
        stiffness: 340.0,
        damping: 20.0,
    };
    /// Leaving: quick and without a bounce (critically damped), about 0.15 s.
    pub const EXIT: Spring = Spring {
        stiffness: 700.0,
        damping: 53.0,
    };

    /// Advances `(x, v)` toward `target` by `dt` seconds. Semi-implicit Euler in steps of at
    /// most 4 ms, so it stays stable at any frame rate.
    pub fn step(self, mut x: f32, mut v: f32, target: f32, dt: f32) -> (f32, f32) {
        let steps = (dt / 0.004).ceil().max(1.0) as u32;
        let h = dt / steps as f32;
        for _ in 0..steps {
            let a = -self.stiffness * (x - target) - self.damping * v;
            v += a * h;
            x += v * h;
        }
        (x, v)
    }
}

/// Whether a value this close to `target` and this slow may snap to it.
fn settled(x: f32, v: f32, target: f32) -> bool {
    let eps = (target.abs() * 1e-4).max(1e-3);
    (x - target).abs() < eps && v.abs() < eps * 20.0
}

#[derive(Clone, Copy, Debug)]
struct SpringState {
    x: f32,
    v: f32,
    time: f64,
}

#[derive(Clone, Copy, Debug)]
struct FadeState {
    from: Color32,
    to: Color32,
    start: f64,
}

/// True when animations are off: View > Reduce motion, or a host that turned egui's own off.
pub fn reduced(ctx: &Context) -> bool {
    ctx.global_style().animation_time <= 0.0
}

/// Turns all motion (egui's and ours) off or back on.
pub fn set_reduced(ctx: &Context, reduce: bool) {
    let time = if reduce { 0.0 } else { EGUI_ANIMATION_TIME };
    ctx.all_styles_mut(|s| s.animation_time = time);
}

fn now(ctx: &Context) -> f64 {
    ctx.input(|i| i.time)
}

/// Springs the value under `id` toward `target`. The first call starts at `start`
/// (`None`: at `target`, so nothing moves until the target changes).
pub fn spring_from(ctx: &Context, id: Id, target: f32, start: Option<f32>, spring: Spring) -> f32 {
    let time = now(ctx);
    let reduce = reduced(ctx);
    let (x, moving) = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_insert_with(id, || SpringState {
            x: start.unwrap_or(target),
            v: 0.0,
            time,
        });
        // A long gap (the widget was hidden) moves at most one 50 ms step.
        let dt = ((time - s.time) as f32).clamp(0.0, 0.05);
        s.time = time;
        if reduce {
            (s.x, s.v) = (target, 0.0);
        } else if dt > 0.0 {
            (s.x, s.v) = spring.step(s.x, s.v, target, dt);
            if settled(s.x, s.v, target) {
                (s.x, s.v) = (target, 0.0);
            }
        }
        (s.x, s.x != target || s.v != 0.0)
    });
    if moving {
        ctx.request_repaint();
    }
    x
}

/// [`spring_from`] starting at the target.
pub fn spring(ctx: &Context, id: Id, target: f32, spring: Spring) -> f32 {
    spring_from(ctx, id, target, None, spring)
}

/// Whether the overlay under `id` ([`presence`]) was still on screen in its last frame:
/// open, or fading out.
pub fn on_screen(ctx: &Context, id: Id) -> bool {
    peek(ctx, id).is_some_and(|x| x > 0.001)
}

/// The spring under `id` as of its last step, without stepping it.
pub fn peek(ctx: &Context, id: Id) -> Option<f32> {
    ctx.data(|d| d.get_temp::<SpringState>(id)).map(|s| s.x)
}

/// Drops the spring under `id` (an overlay that is gone for good).
pub fn forget(ctx: &Context, id: Id) {
    ctx.data_mut(|d| d.remove::<SpringState>(id));
}

/// Jumps the spring under `id` to `value` (for example to replay a pop-in).
pub fn reset(ctx: &Context, id: Id, value: f32) {
    let time = now(ctx);
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            SpringState {
                x: value,
                v: 0.0,
                time,
            },
        );
    });
}

/// How present an overlay is, 0 (gone) to about 1: it pops in while `open` and eases out
/// after it closes, so nothing just vanishes. Draw it while this is above 0, taking no input
/// once it is closing ([`inert`]).
pub fn presence(ctx: &Context, id: Id, open: bool) -> f32 {
    let (target, spring) = if open {
        (1.0, Spring::BOUNCY)
    } else {
        (0.0, Spring::EXIT)
    };
    let x = spring_from(ctx, id, target, Some(0.0), spring);
    if open { x } else { x.max(0.0) }
}

/// Eases from 0 to 1 over `secs` each time `key` changes under `id`: something new (another
/// page, a hover outline, a match) fading in. Always 1 with motion reduced.
pub fn arrive(
    ctx: &Context,
    id: Id,
    key: impl std::hash::Hash + std::fmt::Debug,
    secs: f32,
) -> f32 {
    if reduced(ctx) {
        return 1.0;
    }
    let key = Id::new(key);
    let time = now(ctx);
    let start = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_insert_with(id, || (key, time));
        if s.0 != key {
            *s = (key, time);
        }
        s.1
    });
    let t = ((time - start) as f32 / secs.max(1e-3)).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    egui::emath::easing::cubic_out(t)
}

/// Springs from 0 to 1, with a small overshoot, each time `key` changes under `id`: a pop
/// for something that just became current (the find bar's match).
pub fn pop_on(ctx: &Context, id: Id, key: impl std::hash::Hash + std::fmt::Debug) -> f32 {
    let key = Id::new(key);
    let changed = ctx.data_mut(|d| {
        let slot = id.with("key");
        let old = d.get_temp::<Id>(slot);
        d.insert_temp(slot, key);
        old != Some(key)
    });
    if changed {
        reset(ctx, id, 0.0);
    }
    spring(ctx, id, 1.0, Spring::BOUNCY)
}

/// Makes `ui` ignore input without greying it out: an overlay fading away.
pub fn inert(ui: &mut egui::Ui) {
    ui.style_mut().visuals.disabled_alpha = 1.0;
    ui.disable();
}

/// Springs each edge of `target` (a highlight sliding between rows).
pub fn spring_rect(ctx: &Context, id: Id, target: Rect, spring: Spring) -> Rect {
    let edge = |i: u8, v: f32| spring_from(ctx, id.with(i), v, None, spring);
    Rect::from_min_max(
        egui::pos2(edge(0, target.min.x), edge(1, target.min.y)),
        egui::pos2(edge(2, target.max.x), edge(3, target.max.y)),
    )
}

/// Eases the colour under `id` toward `target` over `secs` (a cubic ease-out).
pub fn fade(ctx: &Context, id: Id, target: Color32, secs: f32) -> Color32 {
    let time = now(ctx);
    let reduce = reduced(ctx);
    let (color, moving) = ctx.data_mut(|d| {
        let s = d.get_temp_mut_or_insert_with(id, || FadeState {
            from: target,
            to: target,
            start: time,
        });
        let t = |s: &FadeState| ((time - s.start) as f32 / secs.max(1e-3)).clamp(0.0, 1.0);
        if s.to != target {
            let shown = s
                .from
                .lerp_to_gamma(s.to, egui::emath::easing::cubic_out(t(s)));
            *s = FadeState {
                from: shown,
                to: target,
                start: time,
            };
        }
        let t = if reduce { 1.0 } else { t(s) };
        (
            s.from
                .lerp_to_gamma(s.to, egui::emath::easing::cubic_out(t)),
            t < 1.0,
        )
    });
    if moving {
        ctx.request_repaint();
    }
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps `spring` at 60 fps from 0 to 1; returns the peak and the frames until it rests.
    fn settle(spring: Spring) -> (f32, usize) {
        let (mut x, mut v, mut peak) = (0.0_f32, 0.0_f32, 0.0_f32);
        for frame in 1..=600 {
            (x, v) = spring.step(x, v, 1.0, 1.0 / 60.0);
            peak = peak.max(x);
            if settled(x, v, 1.0) {
                return (peak, frame);
            }
        }
        panic!("{spring:?} never settled: x {x} v {v}");
    }

    #[test]
    fn springs_settle_quickly_and_only_the_bouncy_one_overshoots() {
        let (peak, frames) = settle(Spring::SMOOTH);
        assert!(peak < 1.01, "smooth overshoot {peak}");
        assert!(frames < 40, "smooth took {frames} frames");
        let (peak, frames) = settle(Spring::BOUNCY);
        assert!((1.02..1.2).contains(&peak), "bouncy peak {peak}");
        assert!(frames < 60, "bouncy took {frames} frames");
    }

    #[test]
    fn a_spring_does_not_depend_on_the_frame_rate() {
        let at = |fps: f32| {
            let (mut x, mut v) = (0.0, 0.0);
            for _ in 0..(fps * 0.1) as usize {
                (x, v) = Spring::BOUNCY.step(x, v, 1.0, 1.0 / fps);
            }
            x
        };
        assert!(
            (at(30.0) - at(120.0)).abs() < 0.02,
            "{} {}",
            at(30.0),
            at(120.0)
        );
    }

    /// Runs one frame at `time` seconds and returns `f`'s value.
    fn frame<T>(ctx: &Context, time: f64, mut f: impl FnMut(&Context) -> T) -> T {
        let mut out = None;
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let _ = ctx.run_ui(input, |ui| out = Some(f(ui.ctx())));
        out.expect("ran")
    }

    #[test]
    fn values_move_over_time_and_jump_when_motion_is_reduced() {
        let ctx = Context::default();
        set_reduced(&ctx, false);
        let id = Id::new("x");
        assert_eq!(
            frame(&ctx, 0.0, |c| spring(c, id, 0.0, Spring::SMOOTH)),
            0.0
        );
        let mid = frame(&ctx, 0.03, |c| spring(c, id, 100.0, Spring::SMOOTH));
        assert!(mid > 0.0 && mid < 100.0, "{mid}");
        let end = frame(&ctx, 2.0, |c| spring(c, id, 100.0, Spring::SMOOTH));
        assert!(end > mid, "{end}");
        set_reduced(&ctx, true);
        assert_eq!(
            frame(&ctx, 2.01, |c| spring(c, id, -5.0, Spring::SMOOTH)),
            -5.0
        );

        let (id, red, blue) = (Id::new("colour"), Color32::RED, Color32::BLUE);
        assert_eq!(
            frame(&ctx, 3.0, |c| fade(c, id, red, 0.3)),
            red,
            "first call"
        );
        set_reduced(&ctx, false);
        assert_eq!(frame(&ctx, 3.1, |c| fade(c, id, blue, 0.3)), red, "starts");
        let mid = frame(&ctx, 3.2, |c| fade(c, id, blue, 0.3));
        assert!(mid != red && mid != blue, "{mid:?}");
        assert_eq!(frame(&ctx, 3.5, |c| fade(c, id, blue, 0.3)), blue);
    }

    #[test]
    fn an_overlay_pops_in_eases_out_and_new_things_arrive_again() {
        let ctx = Context::default();
        set_reduced(&ctx, false);
        let id = Id::new("overlay");
        assert_eq!(
            frame(&ctx, 0.0, |c| presence(c, id, false)),
            0.0,
            "never shown"
        );
        // Frames at 60 fps from `from` to `to`; the last value.
        let run = |from: f64, to: f64, open: bool| {
            let mut t = from;
            let mut x = 0.0;
            while t <= to {
                x = frame(&ctx, t, |c| presence(c, id, open));
                t += 1.0 / 60.0;
            }
            x
        };
        let opening = run(0.016, 0.02, true);
        assert!(opening > 0.0 && opening < 1.0, "pops in: {opening}");
        let open = run(0.03, 1.0, true);
        assert!((open - 1.0).abs() < 0.01, "{open}");
        let leaving = run(1.01, 1.04, false);
        assert!(
            leaving > 0.05 && leaving < 1.0,
            "still there while closing: {leaving}"
        );
        assert_eq!(run(1.05, 1.6, false), 0.0, "then gone");

        let id = Id::new("arrive");
        assert_eq!(frame(&ctx, 2.0, |c| arrive(c, id, 1, 0.2)), 0.0, "starts");
        let mid = frame(&ctx, 2.1, |c| arrive(c, id, 1, 0.2));
        assert!(mid > 0.0 && mid < 1.0, "{mid}");
        assert_eq!(frame(&ctx, 2.5, |c| arrive(c, id, 1, 0.2)), 1.0);
        assert_eq!(
            frame(&ctx, 2.6, |c| arrive(c, id, 2, 0.2)),
            0.0,
            "a new key again"
        );

        let id = Id::new("pop");
        let first = frame(&ctx, 3.0, |c| pop_on(c, id, "a"));
        let mut settled = first;
        for i in 1..=60 {
            settled = frame(&ctx, 3.0 + f64::from(i) / 60.0, |c| pop_on(c, id, "a"));
        }
        assert!(
            first < 0.1 && (settled - 1.0).abs() < 0.01,
            "{first} {settled}"
        );
        assert!(
            frame(&ctx, 4.01, |c| pop_on(c, id, "b")) < 0.2,
            "pops again"
        );

        set_reduced(&ctx, true);
        assert_eq!(frame(&ctx, 5.0, |c| presence(c, Id::new("r"), true)), 1.0);
        assert_eq!(frame(&ctx, 5.0, |c| arrive(c, Id::new("q"), 9, 0.2)), 1.0);
    }
}
