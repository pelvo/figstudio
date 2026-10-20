//! The real `App` driven headlessly with egui_kittest: no window, GPU, display or OS clipboard.
//!
//! `harness()` opens `structure.fig` on the CPU tile path (kittest gives no wgpu state). The
//! sibling files hold the tests: `ui_mcp_tests.rs` (View menu), `ui_copy_tests.rs` (Copy menus,
//! marquee, click and Hand-tool drag), `ui_chrome_tests.rs` (sidebar, file switcher, tool pill,
//! command bar).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Event, OutputCommand, PointerButton, Pos2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use figstudio_fig::Guid;

use crate::App;

#[path = "ui_chrome_tests.rs"]
mod chrome;
#[path = "ui_copy_tests.rs"]
mod copy;
#[path = "ui_pick_cousins.rs"]
mod cousins;
#[path = "ui_inner_tests.rs"]
mod inner;
#[path = "ui_inner_fixture.rs"]
mod inner_fixture;
#[path = "ui_inner_hover_tests.rs"]
mod inner_hover;
#[path = "ui_inner_nested_tests.rs"]
mod inner_nested;
#[path = "ui_pick_tests.rs"]
mod pick;
#[path = "ui_pick_bench.rs"]
mod pick_bench;
#[path = "ui_pick_cache_tests.rs"]
mod pick_cache;
#[path = "ui_text_select_tests.rs"]
mod text_select;
// Without the `mcp` feature there is no View item (mcp_off.rs) and no tokio to test it with.
#[cfg(feature = "mcp")]
#[path = "ui_mcp_tests.rs"]
mod mcp;

pub const FIXTURE: &str = "structure.fig";
pub const SIZE: egui::Vec2 = egui::vec2(1200.0, 800.0);

pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Steps `h` once and appends what the frame put on the clipboard to `copied`: the text, or
/// `image WxH` for a picture. kittest has no OS clipboard; this reads egui's output commands.
pub fn step(h: &mut Harness<'_, App>, copied: &mut Vec<String>) {
    h.step();
    for cmd in &h.output().platform_output.commands {
        match cmd {
            OutputCommand::CopyText(t) => copied.push(t.clone()),
            OutputCommand::CopyImage(i) => {
                copied.push(format!("image {}x{}", i.width(), i.height()))
            }
            _ => {}
        }
    }
}

/// Steps until `done` holds, at most 10 s; the render and load workers need real time.
pub fn wait(
    h: &mut Harness<'_, App>,
    copied: &mut Vec<String>,
    what: &str,
    done: fn(&App) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(h.state()) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        step(h, copied);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The app on `structure.fig`, laid out and fitted (the canvas rect is known).
pub fn harness() -> Harness<'static, App> {
    harness_fixture(FIXTURE, 0.25)
}

pub fn harness_fixture(name: &str, step_dt: f32) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(SIZE)
        .with_step_dt(step_dt)
        .build_eframe(|cc| App::new(cc, vec![fixture(name)]));
    wait(&mut h, &mut Vec::new(), "the file to load", |a| {
        a.tabs
            .first()
            .is_some_and(|t| t.canvas_rect.is_some() && !t.fit_pending)
    });
    h
}

pub fn press(h: &Harness<'_, App>, pos: Pos2, button: PointerButton, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
}

/// Primary press at `from`, a few moves, release at `to`.
pub fn drag(h: &mut Harness<'_, App>, copied: &mut Vec<String>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    press(h, from, PointerButton::Primary, true);
    for i in 1..=4 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
    }
    press(h, to, PointerButton::Primary, false);
    step(h, copied);
}

pub fn right_click(h: &mut Harness<'_, App>, copied: &mut Vec<String>, at: Pos2) {
    h.event(Event::PointerMoved(at));
    press(h, at, PointerButton::Secondary, true);
    press(h, at, PointerButton::Secondary, false);
    step(h, copied);
}

/// Clicks the menu entry `label` and lets the frame that copies it run.
pub fn click_label(h: &mut Harness<'_, App>, copied: &mut Vec<String>, label: &str) {
    h.get_by_label(label).click();
    for _ in 0..4 {
        step(h, copied);
    }
}

/// Screen position of the centre of node `id`'s bounds on the active tab.
pub fn centre_of(h: &Harness<'_, App>, id: Guid) -> Pos2 {
    let tab = &h.state().tabs[0];
    let b = figstudio_render::node_bounds(&tab.doc, id).expect("node has bounds");
    let page = Pos2::new(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
    tab.view
        .to_screen(tab.canvas_rect.expect("laid out").min, page)
}
