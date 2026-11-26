//! Prototype reactions and flow starts made by the editor are what the player reads.

use std::path::PathBuf;

use figstudio_fig::edit::{Action, EditOp, History, ReactionSpec, Trigger, execute_edit_ops};
use figstudio_fig::{Document, Guid};
use figstudio_render::Renderer;
use figstudio_render::prototype::{Screen, flows};

fn run(doc: &mut Document, h: &mut History, op: EditOp) -> Vec<Guid> {
    execute_edit_ops(doc, h, &[op], None, None, false)
        .unwrap()
        .0
        .created_ids
}

fn shape(kind: &str, parent: Guid, x: f64, w: f64, h: f64) -> EditOp {
    EditOp::CreateShape {
        id: None,
        kind: kind.into(),
        parent,
        name: None,
        x,
        y: 10.0,
        width: w,
        height: h,
        corner_radius: 0.0,
        fill_hex: None,
        count: None,
        star_inner_scale: None,
    }
}

#[test]
fn the_player_reads_reactions_and_flow_starts_made_by_the_editor() {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let mut h = History::new(20);
    let page = doc.pages[0];
    let a = run(&mut doc, &mut h, shape("FRAME", page, 5000.0, 200.0, 100.0))[0];
    let b = run(&mut doc, &mut h, shape("FRAME", page, 5300.0, 200.0, 100.0))[0];
    let button = run(&mut doc, &mut h, shape("RECTANGLE", a, 10.0, 50.0, 20.0))[0];
    run(
        &mut doc,
        &mut h,
        EditOp::SetReactions {
            target: button,
            reactions: vec![ReactionSpec {
                trigger: Trigger::OnClick,
                action: Action::Navigate { to: b },
                transition: Some("dissolve".into()),
                duration: Some(0.2),
                easing: None,
            }],
        },
    );
    run(
        &mut doc,
        &mut h,
        EditOp::SetFlowStart {
            frame: a,
            name: Some("Flow A".into()),
        },
    );
    let flow = flows(&doc)
        .into_iter()
        .find(|f| f.start == a)
        .expect("the flow is listed");
    assert_eq!(flow.name, "Flow A");
    let screen = Screen::read(&mut Renderer::new(), &doc, a);
    let link = screen
        .links
        .iter()
        .find(|l| l.destination == Some(b))
        .expect("the link is read");
    assert_eq!(
        (link.trigger, link.action, link.transition),
        ("ON_CLICK", "NAVIGATE", "DISSOLVE")
    );
    assert!(link.step.is_some(), "the player plays it");
}
