//! The linked-room confirmation dialog, driven headlessly: joining must take intent
//! (the safe choice answers Enter alone), Escape declines and is consumed, and the full
//! capability never reaches the screen.
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::mcp_join::{JoinReply, RoomProposal, join_dialog};
use crate::mcp_pair::ExecutorPair;

const EXECUTOR: &str = "ex_000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

struct Ask {
    proposal: Option<RoomProposal>,
    replies: Vec<JoinReply>,
    /// Whether Escape was still unread after the dialog handled the frame.
    escape_after: Vec<bool>,
}

impl eframe::App for Ask {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(proposal) = self.proposal.clone()
            && let Some(reply) = join_dialog(ui, &proposal)
        {
            self.replies.push(reply);
            self.proposal = None;
        }
        self.escape_after
            .push(ui.input(|i| i.key_pressed(egui::Key::Escape)));
    }
}

fn harness() -> Harness<'static, Ask> {
    Harness::builder()
        .with_size(egui::vec2(360.0, 180.0))
        .build_eframe(|_| Ask {
            proposal: Some(RoomProposal::new(
                ExecutorPair::from_executor(EXECUTOR).unwrap(),
            )),
            replies: Vec::new(),
            escape_after: Vec::new(),
        })
}

fn key(h: &mut Harness<'_, Ask>, key: egui::Key) {
    h.event(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
}

#[test]
fn enter_alone_never_joins() {
    let mut h = harness();
    h.run_steps(3);
    key(&mut h, egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().replies, [JoinReply::Stay]);
    assert_eq!(h.state().proposal, None);
}

#[test]
fn tab_then_enter_joins() {
    let mut h = harness();
    h.run_steps(3);
    key(&mut h, egui::Key::Tab);
    h.run_steps(1);
    key(&mut h, egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().replies, [JoinReply::Join]);
}

#[test]
fn escape_declines_and_consumes_the_key() {
    let mut h = harness();
    h.run_steps(3);
    key(&mut h, egui::Key::Escape);
    h.run_steps(1);
    assert_eq!(h.state().replies, [JoinReply::Stay]);
    assert!(!h.state().escape_after.last().unwrap());
}

#[test]
fn clicking_joins() {
    let mut h = harness();
    h.run_steps(3);
    h.get_by_label("Join that room").click();
    h.run_steps(3);
    assert_eq!(h.state().replies, [JoinReply::Join]);
}

#[test]
fn the_screen_shows_the_masked_room_never_the_capability() {
    let mut h = harness();
    h.run_steps(3);
    let room = ExecutorPair::from_executor(EXECUTOR).unwrap().room();
    let masked = RoomProposal::new(ExecutorPair::from_executor(EXECUTOR).unwrap()).masked();
    assert!(h.query_all_by_label_contains(&masked).count() > 0);
    assert_eq!(h.query_all_by_label_contains(&room).count(), 0);
    assert_eq!(h.query_all_by_label_contains(&room[3..40]).count(), 0);
    assert_eq!(h.query_all_by_label_contains(EXECUTOR).count(), 0);
    // The warning names what the link really grants: executor authority.
    assert!(h.query_all_by_label_contains("executor authority").count() > 0);
}
