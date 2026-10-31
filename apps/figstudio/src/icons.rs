//! Line icons painted with egui shapes on a 16-unit grid (crisp at any scale, no icon font),
//! and the icon button the chrome uses.

use egui::epaint::CubicBezierShape;
use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType, vec2,
};

use crate::anim::{self, Spring};
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sidebar,
    Inspector,
    More,
    Search,
    Plus,
    Minus,
    Close,
    Select,
    Hand,
    Zoom,
    Fit,
    Page,
    Frame,
    Text,
    Component,
    Instance,
    Group,
    Rect,
    Ellipse,
    Vector,
    Line,
    Section,
    Slice,
    Image,
    Dot,
    Warning,
    Gear,
    Keyboard,
    Sparkle,
    Info,
    Grid,
    List,
    Play,
    Pause,
}

/// The icon for a node type in the layer tree.
pub fn for_kind(kind: &str) -> Icon {
    match kind {
        "FRAME" => Icon::Frame,
        "SECTION" => Icon::Section,
        "GROUP" => Icon::Group,
        "SYMBOL" => Icon::Component,
        "INSTANCE" => Icon::Instance,
        "TEXT" => Icon::Text,
        "VECTOR" | "BOOLEAN_OPERATION" | "STAR" | "REGULAR_POLYGON" => Icon::Vector,
        "LINE" => Icon::Line,
        "ELLIPSE" => Icon::Ellipse,
        "RECTANGLE" | "ROUNDED_RECTANGLE" => Icon::Rect,
        "SLICE" => Icon::Slice,
        _ => Icon::Dot,
    }
}

/// Maps the 16-unit design grid onto `rect` (centred, square).
#[derive(Clone, Copy)]
struct Grid {
    origin: Pos2,
    scale: f32,
}

impl Grid {
    fn new(rect: Rect) -> Self {
        let scale = rect.width().min(rect.height()) / 16.0;
        Self {
            origin: rect.center() - vec2(8.0, 8.0) * scale,
            scale,
        }
    }
    fn p(self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }
    fn ps(self, points: &[(f32, f32)]) -> Vec<Pos2> {
        points.iter().map(|&(x, y)| self.p(x, y)).collect()
    }
    fn rect(self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(self.p(x0, y0), self.p(x1, y1))
    }
}

/// Paints `icon` into `rect` in `color`.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let g = Grid::new(rect);
    let w = (1.3 * g.scale).max(1.0);
    let stroke = Stroke::new(w, color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([g.p(a.0, a.1), g.p(b.0, b.1)], stroke);
    };
    // A line with round ends (egui strokes have butt caps).
    let capsule = |a: (f32, f32), b: (f32, f32)| {
        line(a, b);
        painter.circle_filled(g.p(a.0, a.1), w / 2.0, color);
        painter.circle_filled(g.p(b.0, b.1), w / 2.0, color);
    };
    let path = |points: &[(f32, f32)], closed: bool| {
        let points = g.ps(points);
        painter.add(if closed {
            Shape::closed_line(points, stroke)
        } else {
            Shape::line(points, stroke)
        });
    };
    let round_rect = |r: Rect, radius: f32| {
        let radius = CornerRadius::from(radius * g.scale);
        painter.rect_stroke(r, radius, stroke, StrokeKind::Middle);
    };
    match icon {
        Icon::Sidebar | Icon::Inspector => {
            round_rect(g.rect(1.5, 2.5, 14.5, 13.5), 2.5);
            let x = if icon == Icon::Sidebar { 6.0 } else { 10.0 };
            line((x, 2.5), (x, 13.5));
        }
        Icon::More => {
            for x in [3.5, 8.0, 12.5] {
                painter.circle_filled(g.p(x, 8.0), 1.3 * g.scale, color);
            }
        }
        Icon::Search | Icon::Zoom => {
            painter.circle_stroke(g.p(7.0, 7.0), 4.6 * g.scale, stroke);
            capsule((10.6, 10.6), (13.8, 13.8));
            if icon == Icon::Zoom {
                line((5.0, 7.0), (9.0, 7.0));
                line((7.0, 5.0), (7.0, 9.0));
            }
        }
        Icon::Plus => {
            capsule((8.0, 3.0), (8.0, 13.0));
            capsule((3.0, 8.0), (13.0, 8.0));
        }
        Icon::Minus => capsule((3.0, 8.0), (13.0, 8.0)),
        Icon::Close => {
            capsule((4.5, 4.5), (11.5, 11.5));
            capsule((11.5, 4.5), (4.5, 11.5));
        }
        Icon::Select => path(
            &[
                (4.0, 2.0),
                (4.0, 13.4),
                (6.9, 10.7),
                (9.0, 14.6),
                (10.9, 13.7),
                (8.9, 9.9),
                (12.8, 9.7),
            ],
            true,
        ),
        Icon::Hand => {
            for (x, top, bottom) in [
                (4.6, 6.0, 10.0),
                (7.0, 3.2, 8.6),
                (9.4, 3.6, 8.6),
                (11.8, 5.6, 9.6),
            ] {
                capsule((x, top), (x, bottom));
            }
            path(
                &[
                    (4.6, 10.0),
                    (5.4, 12.6),
                    (7.2, 14.4),
                    (10.4, 14.4),
                    (11.9, 12.8),
                    (11.8, 9.6),
                ],
                false,
            );
        }
        Icon::Fit => {
            for (a, b, c) in [
                ((2.5, 6.0), (2.5, 2.5), (6.0, 2.5)),
                ((10.0, 2.5), (13.5, 2.5), (13.5, 6.0)),
                ((13.5, 10.0), (13.5, 13.5), (10.0, 13.5)),
                ((6.0, 13.5), (2.5, 13.5), (2.5, 10.0)),
            ] {
                path(&[a, b, c], false);
            }
        }
        Icon::Page => {
            path(
                &[
                    (3.5, 1.5),
                    (9.5, 1.5),
                    (12.5, 4.5),
                    (12.5, 14.5),
                    (3.5, 14.5),
                ],
                true,
            );
            path(&[(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)], false);
        }
        Icon::Frame => {
            line((5.5, 1.5), (5.5, 14.5));
            line((10.5, 1.5), (10.5, 14.5));
            line((1.5, 5.5), (14.5, 5.5));
            line((1.5, 10.5), (14.5, 10.5));
        }
        Icon::Text => {
            capsule((3.0, 3.0), (13.0, 3.0));
            capsule((8.0, 3.0), (8.0, 13.5));
        }
        Icon::Component => {
            for (x, y) in [(8.0, 3.6), (3.6, 8.0), (12.4, 8.0), (8.0, 12.4)] {
                let d = 2.3;
                path(&[(x, y - d), (x + d, y), (x, y + d), (x - d, y)], true);
            }
        }
        Icon::Instance => path(&[(8.0, 2.0), (14.0, 8.0), (8.0, 14.0), (2.0, 8.0)], true),
        Icon::Group => {
            let pts = g.ps(&[
                (2.5, 2.5),
                (13.5, 2.5),
                (13.5, 13.5),
                (2.5, 13.5),
                (2.5, 2.5),
            ]);
            painter.extend(Shape::dashed_line(
                &pts,
                stroke,
                2.2 * g.scale,
                1.6 * g.scale,
            ));
        }
        Icon::Rect => round_rect(g.rect(2.5, 3.5, 13.5, 12.5), 1.5),
        Icon::Ellipse => {
            painter.circle_stroke(g.p(8.0, 8.0), 5.6 * g.scale, stroke);
        }
        Icon::Vector => {
            painter.add(CubicBezierShape::from_points_stroke(
                [
                    g.p(2.8, 12.5),
                    g.p(5.0, 2.0),
                    g.p(11.0, 14.0),
                    g.p(13.2, 3.5),
                ],
                false,
                Color32::TRANSPARENT,
                stroke,
            ));
            painter.circle_filled(g.p(2.8, 12.5), 1.5 * g.scale, color);
            painter.circle_filled(g.p(13.2, 3.5), 1.5 * g.scale, color);
        }
        Icon::Line => capsule((3.0, 13.0), (13.0, 3.0)),
        Icon::Section => path(
            &[
                (2.0, 14.0),
                (2.0, 2.5),
                (7.0, 2.5),
                (8.0, 5.0),
                (14.0, 5.0),
                (14.0, 14.0),
            ],
            true,
        ),
        Icon::Slice => {
            round_rect(g.rect(2.5, 2.5, 13.5, 13.5), 1.0);
            line((2.5, 13.5), (13.5, 2.5));
        }
        Icon::Image => {
            round_rect(g.rect(2.0, 3.0, 14.0, 13.0), 1.5);
            path(
                &[
                    (2.5, 12.0),
                    (6.5, 8.0),
                    (9.5, 11.0),
                    (11.0, 9.5),
                    (13.5, 12.0),
                ],
                false,
            );
            painter.circle_filled(g.p(10.8, 6.0), 1.3 * g.scale, color);
        }
        Icon::Dot => {
            painter.circle_filled(g.p(8.0, 8.0), 1.8 * g.scale, color);
        }
        Icon::Gear => {
            painter.circle_stroke(g.p(8.0, 8.0), 4.4 * g.scale, stroke);
            painter.circle_stroke(g.p(8.0, 8.0), 1.8 * g.scale, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let (s, c) = a.sin_cos();
                capsule(
                    (8.0 + c * 4.6, 8.0 + s * 4.6),
                    (8.0 + c * 6.6, 8.0 + s * 6.6),
                );
            }
        }
        Icon::Keyboard => {
            round_rect(g.rect(1.5, 4.0, 14.5, 12.5), 2.0);
            for (y, xs) in [(6.6, [4.2, 6.4, 8.6, 10.8]), (8.6, [5.2, 7.4, 9.6, 11.8])] {
                for x in xs {
                    painter.circle_filled(g.p(x, y), 0.75 * g.scale, color);
                }
            }
            capsule((5.5, 10.6), (10.5, 10.6));
        }
        Icon::Sparkle => {
            path(
                &[
                    (7.5, 1.5),
                    (8.9, 6.1),
                    (13.5, 7.5),
                    (8.9, 8.9),
                    (7.5, 13.5),
                    (6.1, 8.9),
                    (1.5, 7.5),
                    (6.1, 6.1),
                ],
                true,
            );
            capsule((13.0, 11.0), (13.0, 14.0));
            capsule((11.5, 12.5), (14.5, 12.5));
        }
        Icon::Info => {
            painter.circle_stroke(g.p(8.0, 8.0), 6.2 * g.scale, stroke);
            painter.circle_filled(g.p(8.0, 4.9), 0.95 * g.scale, color);
            capsule((8.0, 7.3), (8.0, 11.4));
        }
        Icon::Grid => {
            for (x, y) in [(2.0, 2.0), (9.0, 2.0), (2.0, 9.0), (9.0, 9.0)] {
                round_rect(g.rect(x, y, x + 5.0, y + 5.0), 1.3);
            }
        }
        Icon::List => {
            for y in [3.5, 8.0, 12.5] {
                painter.circle_filled(g.p(2.6, y), 0.95 * g.scale, color);
                capsule((5.5, y), (14.0, y));
            }
        }
        Icon::Play => path(&[(5.0, 3.0), (13.0, 8.0), (5.0, 13.0)], true),
        Icon::Pause => {
            capsule((5.5, 3.5), (5.5, 12.5));
            capsule((10.5, 3.5), (10.5, 12.5));
        }
        Icon::Warning => {
            path(&[(8.0, 2.0), (14.5, 13.5), (1.5, 13.5)], true);
            capsule((8.0, 6.2), (8.0, 9.4));
            painter.circle_filled(g.p(8.0, 11.6), 0.95 * g.scale, color);
        }
    }
}

/// A chevron in `rect`: pointing right at `turn` 0, down at 1 (in between while it rotates).
pub fn chevron(painter: &Painter, rect: Rect, color: Color32, turn: f32) {
    let g = Grid::new(rect);
    let angle = turn * std::f32::consts::FRAC_PI_2;
    let (sin, cos) = angle.sin_cos();
    let c = g.p(8.0, 8.0);
    let rot = |x: f32, y: f32| {
        let v = vec2(x - 8.0, y - 8.0) * g.scale;
        c + vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    };
    let stroke = Stroke::new((1.4 * g.scale).max(1.0), color);
    painter.add(Shape::line(
        vec![rot(6.0, 3.5), rot(10.5, 8.0), rot(6.0, 12.5)],
        stroke,
    ));
}

/// Hover fill and press dip for a custom-painted widget: returns `(hover 0..1, press 0..1)`.
pub fn feedback(ui: &Ui, response: &Response) -> (f32, f32) {
    let ctx = ui.ctx();
    let hover = ctx.animate_bool_with_time(
        response.id.with("hover"),
        response.hovered() && ui.is_enabled(),
        0.12,
    );
    let press = anim::spring(
        ctx,
        response.id.with("press"),
        if response.is_pointer_button_down_on() {
            1.0
        } else {
            0.0
        },
        Spring::BOUNCY,
    );
    (hover, press.clamp(-0.5, 1.5))
}

/// A square icon button with a tooltip and an accessible `label`. Hover fades a soft fill in,
/// a press dips it, and `selected` keeps it lit.
pub fn button(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    sized_button(ui, icon, label, selected, 28.0)
}

pub fn sized_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool, side: f32) -> Response {
    raw_button(ui, icon, label, selected, side).on_hover_text(label)
}

/// [`sized_button`] without the tooltip (for a button that opens a menu).
pub fn raw_button(ui: &mut Ui, icon: Icon, label: &str, selected: bool, side: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    response
        .widget_info(|| WidgetInfo::selected(WidgetType::Button, ui.is_enabled(), selected, label));
    if ui.is_rect_visible(rect) {
        let p = theme::of(ui);
        let (hover, press) = feedback(ui, &response);
        let r = rect.shrink(press * 1.5);
        let fill = if selected {
            p.selected
        } else {
            p.hover.gamma_multiply(hover)
        };
        ui.painter().rect_filled(r, CornerRadius::same(8), fill);
        let color = if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        let color = if ui.is_enabled() { color } else { p.text_faint };
        let icon_rect = Rect::from_center_size(r.center(), vec2(16.0, 16.0) * (r.width() / side));
        paint(ui.painter(), icon, icon_rect, color);
    }
    response
}

/// Paints the `selected` / hover background of a row: an Arc-style pill.
pub fn row_fill(ui: &Ui, rect: Rect, hover: f32, selected: bool) {
    let p = theme::of(ui);
    let fill = if selected {
        p.selected
    } else {
        p.hover.gamma_multiply(hover)
    };
    if fill.a() > 0 {
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn the_grid_centres_the_icon_in_a_wide_rect() {
        let g = Grid::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(64.0, 32.0)));
        assert_eq!(g.p(0.0, 0.0), pos2(16.0, 0.0));
        assert_eq!(g.p(16.0, 16.0), pos2(48.0, 32.0));
    }
}
