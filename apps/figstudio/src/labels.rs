//! Names of a page's top-level frames and sections, drawn on the canvas like Figma does.
//! They are editor chrome, not part of the document, so the renderer never draws them.

use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Vec2};
use figstudio_fig::{Document, Guid, Node};
use figstudio_render::geom;
use figstudio_render::kurbo::Point;

#[path = "labels_backdrop.rs"]
mod backdrop;
#[path = "labels_named.rs"]
mod candidates;
#[path = "labels_quad.rs"]
mod quad;
#[path = "labels_spatial.rs"]
mod spatial;

use crate::{
    icons::{self, Icon},
    tab::View,
    theme,
};

/// Smallest on-screen width (points) that still gets a label. Figma keeps section names far
/// out but drops frame names once frames get small: measured against Figma, frames about
/// 270 pt wide keep their name and frames about 180 pt wide lose it.
const MIN_SECTION_WIDTH: f32 = 40.0;
const MIN_FRAME_WIDTH: f32 = 220.0;
/// Bound on labels laid out per frame, so pages with thousands of frames stay fast.
const MAX_LABELS: usize = 2_000;
const ICON_SIZE: f32 = 16.0;
const ICON_GAP: f32 = 4.0;

/// A label to draw: its frame, text, anchor (bottom-left, screen space) and whether it is
/// a section.
#[derive(Debug)]
pub struct Label {
    pub id: Guid,
    pub text: String,
    pub at: Pos2,
    pub max_width: f32,
    pub section: bool,
    pub icon: Option<Icon>,
    pub color: Color32,
    layout: Option<(std::sync::Arc<egui::Galley>, Rect, Pos2)>,
}

/// Canvas colours follow the page background, which can differ from the app's theme.
fn style(node: &Node, background: Color32) -> (Option<Icon>, Color32) {
    let palette = if theme::contrast(theme::DARK.component, background)
        > theme::contrast(theme::LIGHT.component, background)
    {
        theme::DARK
    } else {
        theme::LIGHT
    };
    match node.kind.as_str() {
        "SECTION" => (None, Color32::WHITE),
        "SYMBOL" | "COMPONENT_SET" => (Some(Icon::Component), palette.component),
        "INSTANCE" => (Some(Icon::Instance), palette.component),
        _ if node.data.flag("isStateGroup") == Some(true) => {
            (Some(Icon::Component), palette.component)
        }
        _ => {
            let grey = Color32::from_gray(if palette.dark { 179 } else { 92 });
            let color = if theme::contrast(grey, background) >= 4.5 {
                grey
            } else if theme::contrast(Color32::WHITE, background) >= 4.5 {
                Color32::WHITE
            } else {
                Color32::BLACK
            };
            (None, color)
        }
    }
}

/// The frames and sections whose names the canvas shows (when they are big enough on
/// screen): the page's top-level ones, and those directly inside its sections. Page order.
pub fn named(doc: &Document, page: Guid) -> Vec<Guid> {
    candidates::collect(doc, page)
        .into_iter()
        .map(|n| n.id)
        .collect()
}

/// Labels for the page's top-level frames, and for frames directly inside its sections.
/// Like Figma, a label that would cover another frame or label is not shown.
pub fn labels(
    painter: &egui::Painter,
    doc: &Document,
    page: Guid,
    view: View,
    origin: Pos2,
    screen: Rect,
) -> Vec<Label> {
    let mut out = Vec::new();
    let page_bg = crate::canvas::to_color32(figstudio_render::page_background(doc, page));
    let named = candidates::collect(doc, page);
    let backdrops = backdrops(doc, &named, page_bg);
    let frames = blockers(doc, &named, view, origin, screen);
    let mut placed = spatial::Grid::default();
    for (index, candidate) in named.iter().enumerate() {
        let id = candidate.id;
        if out.len() >= MAX_LABELS {
            break;
        }
        let Some(node) = doc.node(id) else { continue };
        let section = node.kind == "SECTION";
        let tf = candidate.world;
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
            let (icon, color) = style(node, backdrops[index]);
            let mut label = Label {
                id,
                text: node.name.clone(),
                at,
                max_width: width,
                section,
                icon,
                color,
                layout: None,
            };
            let shaped = layout(painter, &label);
            let r = shaped.1;
            label.layout = Some(shaped);
            let blocked = placed.any(r, |_: &()| true)
                || frames.any(r, |&(start, end, points)| {
                    !(start <= index && index < end) && quad::overlaps(r, &points)
                });
            if !blocked {
                placed.insert(r, ());
                out.push(label);
            }
        }
    }
    out
}

/// The colour under each label: its enclosing section's fill over that section's backdrop,
/// or the page. Parents precede their children in `named`, so one pass resolves nesting.
fn backdrops(doc: &Document, named: &[candidates::Named], page: Color32) -> Vec<Color32> {
    let mut surface: Vec<Color32> = Vec::with_capacity(named.len());
    let mut under = Vec::with_capacity(named.len());
    for candidate in named {
        let back = candidate.parent.map_or(page, |p| surface[p]);
        under.push(back);
        let fills = doc
            .node(candidate.id)
            .and_then(|n| n.data.get("fillPaints"))
            .map(figstudio_fig::Val::items)
            .unwrap_or_default();
        // shortcut: image fills keep the backdrop below; sample images if contrast needs it.
        let top = fills.iter().fold(back, |acc, p| {
            if p.flag("visible") == Some(false) {
                return acc;
            }
            backdrop::color_for(p).map_or(acc, |c| {
                let [r, g, b, a] = c.components;
                let solid = crate::canvas::to_color32(figstudio_render::color::AlphaColor::new([
                    r, g, b, 1.0,
                ]));
                acc.lerp_to_gamma(solid, a.clamp(0.0, 1.0))
            })
        });
        surface.push(top);
    }
    under
}

fn blockers(
    doc: &Document,
    named: &[candidates::Named],
    view: View,
    origin: Pos2,
    screen: Rect,
) -> spatial::Grid<(usize, usize, [Pos2; 4])> {
    let mut frames = spatial::Grid::default();
    for (index, candidate) in named.iter().enumerate() {
        let Some(node) = doc.node(candidate.id) else {
            continue;
        };
        let (w, h) = geom::size(&node.data);
        let b = geom::transformed_bounds(
            candidate.world,
            figstudio_render::kurbo::Rect::new(0.0, 0.0, w, h),
        );
        let min = view.to_screen(origin, Pos2::new(b.x0 as f32, b.y0 as f32));
        let max = view.to_screen(origin, Pos2::new(b.x1 as f32, b.y1 as f32));
        let r = Rect::from_min_max(min, max);
        if r.intersects(screen.expand(64.0)) {
            let points = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| {
                let p = candidate.world * Point::new(x, y);
                view.to_screen(origin, Pos2::new(p.x as f32, p.y as f32))
            });
            frames.insert(r, (index, candidate.end, points));
        }
    }
    frames
}

#[cfg(test)]
thread_local! {
    static SHAPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Text, full label bounds (including icon or pill) and text position.
fn layout(painter: &egui::Painter, l: &Label) -> (std::sync::Arc<egui::Galley>, Rect, Pos2) {
    #[cfg(test)]
    SHAPED.set(SHAPED.get() + 1);
    let font = FontId::proportional(if l.section { 13.0 } else { 12.0 });
    let leading = if l.icon.is_some() {
        ICON_SIZE + ICON_GAP
    } else {
        0.0
    };
    let mut job = egui::text::LayoutJob::simple_singleline(l.text.clone(), font, l.color);
    job.wrap.max_width = (l.max_width - leading).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    let pad = if l.section {
        Vec2::new(6.0, 3.0)
    } else {
        Vec2::ZERO
    };
    let height = galley
        .size()
        .y
        .max(if l.icon.is_some() { ICON_SIZE } else { 0.0 });
    let size = Vec2::new(galley.size().x + leading, height) + pad * 2.0;
    let bounds = Rect::from_min_size(l.at - Vec2::new(0.0, size.y + 4.0), size);
    let text_at = bounds.min + pad + Vec2::new(leading, (height - galley.size().y) / 2.0);
    (galley, bounds, text_at)
}

/// Where a label is drawn on screen (its pill, for a section), including its icon.
pub fn rect(painter: &egui::Painter, l: &Label) -> Rect {
    l.layout
        .as_ref()
        .map_or_else(|| layout(painter, l).1, |shaped| shaped.1)
}

/// Draws section pills, grey frame names and purple component/instance names with icons.
pub fn paint(painter: &egui::Painter, labels: &[Label]) {
    for l in labels {
        let (galley, bounds, text_at) = l.layout.clone().unwrap_or_else(|| layout(painter, l));
        if l.section {
            painter.rect_filled(bounds, CornerRadius::same(4), Color32::from_gray(20));
        }
        if let Some(icon) = l.icon {
            let at = Pos2::new(bounds.left(), bounds.center().y - ICON_SIZE / 2.0);
            icons::paint(
                painter,
                icon,
                Rect::from_min_size(at, Vec2::splat(ICON_SIZE)),
                l.color,
            );
        }
        painter.galley(text_at, galley, l.color);
    }
}

#[cfg(test)]
#[path = "labels_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "labels_bench.rs"]
mod bench;

#[cfg(test)]
#[path = "labels_work_tests.rs"]
mod work_tests;

#[cfg(test)]
#[path = "labels_rotation_tests.rs"]
mod rotation_tests;

#[cfg(test)]
#[path = "labels_backdrop_tests.rs"]
mod backdrop_tests;
