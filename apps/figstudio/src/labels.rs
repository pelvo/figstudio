//! Names of a page's top-level frames and sections, drawn on the canvas like Figma does.
//! They are editor chrome, not part of the document, so the renderer never draws them.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Vec2};
use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::Point;
use figstudio_render::{absolute_transform, geom};

use crate::tab::View;

/// Smallest on-screen width (points) that still gets a label. Figma keeps section names far
/// out but drops frame names once frames get small: measured against Figma, frames about
/// 270 pt wide keep their name and frames about 180 pt wide lose it.
const MIN_SECTION_WIDTH: f32 = 40.0;
const MIN_FRAME_WIDTH: f32 = 220.0;
/// Bound on labels laid out per frame, so pages with thousands of frames stay fast.
const MAX_LABELS: usize = 2_000;

/// A label to draw: its text, anchor (bottom-left, screen space) and whether it is a section.
pub struct Label {
    pub text: String,
    pub at: Pos2,
    pub max_width: f32,
    pub section: bool,
}

/// Labels for the page's top-level frames, and for frames directly inside its sections.
pub fn labels(doc: &Document, page: Guid, view: View, origin: Pos2, screen: Rect) -> Vec<Label> {
    let mut out = Vec::new();
    let mut stack: Vec<Guid> = doc.children(page).iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if out.len() >= MAX_LABELS {
            break;
        }
        let Some(node) = doc.node(id) else { continue };
        let section = node.kind == "SECTION";
        if !(section || geom::is_container(&node.kind)) || node.data.flag("visible") == Some(false)
        {
            continue;
        }
        let tf = absolute_transform(doc, id);
        let (w, _) = geom::size(&node.data);
        let a = tf * Point::new(0.0, 0.0);
        let b = tf * Point::new(w, 0.0);
        let at = view.to_screen(origin, Pos2::new(a.x as f32, a.y as f32));
        let width = (view.to_screen(origin, Pos2::new(b.x as f32, b.y as f32)) - at).length();
        let min = if section {
            MIN_SECTION_WIDTH
        } else {
            MIN_FRAME_WIDTH
        };
        if width >= min && screen.expand(width).contains(at) {
            out.push(Label {
                text: node.name.clone(),
                at,
                max_width: width,
                section,
            });
        }
        if section {
            stack.extend(doc.children(id).iter().rev().copied());
        }
    }
    out
}

/// Draws the labels: sections as a dark pill, frames as plain grey text, like Figma.
pub fn paint(painter: &egui::Painter, labels: &[Label]) {
    for l in labels {
        let font = FontId::proportional(if l.section { 13.0 } else { 12.0 });
        let color = if l.section {
            Color32::WHITE
        } else {
            Color32::from_gray(150)
        };
        let mut job = egui::text::LayoutJob::simple_singleline(l.text.clone(), font, color);
        job.wrap.max_width = l.max_width;
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('…');
        let galley = painter.layout_job(job);
        let pad = if l.section {
            Vec2::new(6.0, 3.0)
        } else {
            Vec2::ZERO
        };
        let min = l.at - Vec2::new(0.0, galley.size().y + pad.y * 2.0 + 4.0);
        if l.section {
            let bg = Rect::from_min_size(min, galley.size() + pad * 2.0);
            painter.rect_filled(bg, CornerRadius::same(4), Color32::from_gray(20));
        }
        painter.galley(min + pad, galley, color);
    }
}

#[cfg(test)]
mod tests {
    use super::labels;
    use crate::tab::View;
    use egui::{Pos2, Rect, Vec2};
    use figstudio_fig::{Document, Guid, Node, Obj, Val};

    fn node(n: u32, parent: u32, kind: &str, name: &str, x: f64, w: f64) -> Node {
        let mut o = Obj::new();
        let guid = |n: u32| Guid(9, n).to_val();
        let mut pi = Obj::new();
        pi.insert("guid", guid(parent));
        pi.insert("position", Val::Str(format!("{n:03}")));
        let mut size = Obj::new();
        size.insert("x", Val::Float(w));
        size.insert("y", Val::Float(50.0));
        let mut tf = Obj::new();
        tf.insert("m00", Val::Float(1.0));
        tf.insert("m11", Val::Float(1.0));
        tf.insert("m02", Val::Float(x));
        o.insert("guid", guid(n));
        o.insert("type", Val::Str(kind.into()));
        o.insert("name", Val::Str(name.into()));
        o.insert("parentIndex", Val::Obj(pi));
        o.insert("size", Val::Obj(size));
        o.insert("transform", Val::Obj(tf));
        Node::from_data(Val::Obj(o)).unwrap()
    }

    #[test]
    fn top_level_frames_and_sections_and_their_frames_get_labels() {
        let mut doc =
            Document::from_bytes(include_bytes!("../../../tests/fixtures/structure.fig")).unwrap();
        doc.set_nodes(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "SECTION", "Section A", 0.0, 400.0),
            node(3, 2, "FRAME", "Frame in section", 10.0, 300.0),
            node(4, 3, "FRAME", "Nested frame", 0.0, 300.0),
            node(5, 1, "FRAME", "Top frame", 500.0, 300.0),
            node(6, 1, "RECTANGLE", "Shape", 900.0, 300.0),
            node(7, 1, "FRAME", "Small frame", 1300.0, 200.0),
        ])
        .unwrap();
        let view = View {
            pan: Vec2::ZERO,
            zoom: 1.0,
        };
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(2000.0));
        let names: Vec<String> = labels(&doc, Guid(9, 1), view, Pos2::ZERO, screen)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(names, ["Section A", "Frame in section", "Top frame"]);
    }
}
