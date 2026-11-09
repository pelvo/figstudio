//! Motion playback in the renderer: a [`Motion`] gives poses, the walk applies them and
//! delays layers inside instances.
use std::sync::{Arc, Mutex};

use figstudio_fig::{Document, Guid, Node, Val};
use figstudio_render::kurbo::Vec2;
use figstudio_render::motion::{Motion, Playhead, Pose};
use figstudio_render::{Renderer, node_bounds};

fn open(name: &str) -> Document {
    Document::open(format!(
        "{}/../../tests/fixtures/{name}.fig",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Moves one layer right by the playhead time (µs as px); records the delay every layer is
/// asked with; instances delay their contents by 7 µs.
struct Stub {
    moved: Guid,
    seen: Mutex<Vec<(Guid, f64)>>,
}

impl Motion for Stub {
    fn pose(&self, _: &Document, node: &Node, _: &Val, delay: f64, time: f64) -> Option<Pose> {
        self.seen.lock().unwrap().push((node.id, delay));
        (node.id == self.moved).then(|| Pose {
            translate: Vec2::new(time, 0.0),
            ..Pose::default()
        })
    }
    fn instance_delay(&self, data: &Val) -> f64 {
        if data.get("symbolData").is_some() {
            7.0
        } else {
            0.0
        }
    }
}

fn opaque_columns(doc: &Document, frame: Guid, playhead: Option<Playhead>) -> Vec<usize> {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r.playhead = playhead;
    let f = r
        .render_export_complete(doc, frame, 1.0, None, |_| None)
        .unwrap();
    let w = usize::from(f.pixmap.width());
    let px = f.pixmap.data_as_u8_slice();
    // Columns of the first row of the moved square that differ from white.
    let y = 60;
    (0..w)
        .filter(|&x| {
            let i = (y * w + x) * 4;
            px[i] < 240 && px[i + 3] > 0
        })
        .collect()
}

#[test]
fn a_pose_moves_the_layer_and_instances_delay_their_contents() {
    let doc = open("structure");
    // "Groups" frame; its first square is 1:3 at x 37 (100 px wide, grey).
    let frame = Guid(1, 2);
    let square = doc
        .children(Guid(1, 53))
        .first()
        .copied()
        .expect("a square in the group");
    let still = opaque_columns(&doc, frame, None);
    let stub = Arc::new(Stub {
        moved: square,
        seen: Mutex::new(Vec::new()),
    });
    let moved = opaque_columns(
        &doc,
        frame,
        Some(Playhead {
            motion: stub.clone(),
            time_us: 30.0,
        }),
    );
    let b = node_bounds(&doc, square).unwrap();
    assert!(!still.is_empty(), "{b:?}");
    assert_eq!(moved.first().map(|x| x - 30), still.first().copied());

    // Inside the instances of "Symbols and images", layers are asked with their delay.
    let stub = Arc::new(Stub {
        moved: Guid(0, 0),
        seen: Mutex::new(Vec::new()),
    });
    let mut r = Renderer::new();
    r.system_fonts = false;
    r.playhead = Some(Playhead {
        motion: stub.clone(),
        time_us: 0.0,
    });
    r.render_export_complete(&doc, Guid(1, 4), 1.0, None, |_| None);
    let seen = stub.seen.lock().unwrap();
    let component_children = doc.children(Guid(1, 37));
    assert!(
        seen.iter()
            .any(|(id, d)| component_children.contains(id) && *d == 7.0)
    );
    assert!(seen.iter().any(|(id, d)| *id == Guid(1, 38) && *d == 0.0));
}
