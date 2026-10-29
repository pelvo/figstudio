//! Motion playback (`motion::Player`): layer values at the playhead, checked against what
//! Figma's Motion panel shows for the public motion fixture.
use std::f64::consts::PI;
use std::sync::Arc;

use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::motion::Player;
use figstudio_render::motion::{Motion, Playhead, Pose};
use figstudio_render::{Renderer, node_bounds};

fn doc() -> Document {
    Document::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/motion.fig"
    ))
    .unwrap()
}

fn pose(doc: &Document, p: &Player, id: Guid, ms: f64) -> Pose {
    let n = doc.node(id).unwrap();
    p.pose(doc, n, &n.data, 0.0, ms * 1e3).unwrap()
}

#[test]
fn the_rotating_object_turns_and_moves_as_figma_shows() {
    let doc = doc();
    let p = Player::new(&doc);
    // "Object": Clockwise 360°, delay 100 ms, 700 ms, linear; position 0 to 901 ms, ease out.
    let object = Guid(1, 1106);
    let at = |ms| pose(&doc, &p, object, ms);
    assert!((at(50.0).rotation + 2.0 * PI).abs() < 1e-5);
    assert!((at(450.0).rotation + PI).abs() < 1e-5);
    assert!(at(800.0).rotation.abs() < 1e-5);
    // Ease out: further than linear early on, at rest at the end.
    let end = at(901.0).translate.x;
    assert!((end - 500.0).abs() < 1e-3, "{end}");
    assert!(at(100.0).translate.x > end * 100.0 / 901.0 * 1.4);
    // The 1 s timeline loops (only its duration is stored, which Figma plays as a loop).
    assert_eq!(p.timeline(&doc, Guid(1, 1105)), (1_000_000.0, "loop"));
    assert!((at(1200.0).translate.x - at(200.0).translate.x).abs() < 1e-9);
}

#[test]
fn a_spring_overshoots_and_settles() {
    let doc = doc();
    let p = Player::new(&doc);
    // The custom spring (stiffness 100, damping 6.5) scales 0.1 up to 1.
    let scale = |ms| pose(&doc, &p, Guid(1, 1041), ms).scale.x;
    assert!((scale(0.0) - 0.1).abs() < 1e-6);
    // Figma's Motion panel (Scale %) for this layer: the spring fills its 1 s segment.
    for (ms, figma) in [
        (50.0, 49.76),
        (150.0, 130.47),
        (300.0, 89.69),
        (450.0, 103.49),
    ] {
        let ours = scale(ms) * 100.0;
        assert!(
            (ours - figma).abs() < 0.05,
            "{ms} ms: ours {ours}, Figma {figma}"
        );
    }
    assert!((scale(5000.0) - 1.0).abs() < 1e-6);
}

#[test]
fn a_played_render_moves_the_object() {
    let doc = doc();
    let frame = Guid(1, 1105);
    let ink_left = |time_us: f64| {
        let mut r = Renderer::new();
        r.system_fonts = false;
        r.playhead = Some(Playhead {
            motion: Arc::new(Player::new(&doc)),
            time_us,
        });
        let viewport = figstudio_render::Viewport {
            width: 1200,
            height: 600,
            transform: figstudio_render::kurbo::Affine::translate(
                -node_bounds(&doc, frame).unwrap().origin().to_vec2(),
            ),
            background: None,
        };
        let f = r.render_complete(&doc, frame, &viewport, |_| None);
        let (w, h) = (
            usize::from(f.pixmap.width()),
            usize::from(f.pixmap.height()),
        );
        let px = f.pixmap.data_as_u8_slice();
        // The object is the only saturated colour in the frame.
        (0..w)
            .find(|&x| {
                (0..h).any(|y| {
                    let i = (y * w + x) * 4;
                    let (r, g, b) = (px[i], px[i + 1], px[i + 2]);
                    r.max(g).max(b) - r.min(g).min(b) > 80
                })
            })
            .unwrap()
    };
    let (start, end) = (ink_left(0.0), ink_left(950_000.0));
    assert!(
        (end as f64 - start as f64 - 500.0).abs() <= 2.0,
        "{start} {end}"
    );
}
