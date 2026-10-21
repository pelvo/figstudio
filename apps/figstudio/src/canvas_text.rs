//! Read-only text gestures use renderer-resolved layout, including instance overrides.
use crate::{tab::Tab, text_select, theme};
use egui::{Pos2, Stroke};
use figstudio_render::{
    geom,
    kurbo::{Point, Rect},
};

fn at(tab: &Tab, origin: Pos2, pos: Pos2) -> Option<usize> {
    let (_, tf, data) = tab.text_layout()?;
    if !tf.determinant().is_finite() || tf.determinant().abs() < f64::EPSILON {
        return None;
    }
    let p = tab.view.to_page(origin, pos);
    let local = tf.inverse() * Point::new(f64::from(p.x), f64::from(p.y));
    text_select::char_at(&data, local.x, local.y).map(|(i, _)| i)
}

pub fn input(
    response: &egui::Response,
    tab: &mut Tab,
    origin: Pos2,
    enabled: bool,
    enter_text: bool,
) {
    if !enabled {
        return;
    }
    if response.clicked()
        && let Some(selection) = tab.text_select
        && let Some(index) = response
            .interact_pointer_pos()
            .and_then(|p| at(tab, origin, p))
    {
        tab.text_select = Some(text_select::Selection {
            anchor: index,
            head: index,
            ..selection
        });
    }
    if response.double_clicked()
        && enter_text
        && let Some((id, _, data)) = tab.text_layout()
    {
        let index = response
            .interact_pointer_pos()
            .and_then(|p| at(tab, origin, p))
            .unwrap_or(0);
        let head = if text_select::range_rects(&data, 0, 0).is_none() {
            text_select::characters(&data).len()
        } else {
            index
        };
        tab.marquee.clear();
        tab.text_select = Some(text_select::Selection {
            node: id,
            anchor: index,
            head,
        });
    }
    if response.drag_started_by(egui::PointerButton::Primary)
        && let Some(selection) = tab.text_select
        && let Some(index) = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .and_then(|p| at(tab, origin, p))
    {
        tab.text_select = Some(text_select::Selection {
            anchor: index,
            head: index,
            ..selection
        });
    }
    if response.dragged_by(egui::PointerButton::Primary)
        && let Some(selection) = tab.text_select
        && let Some(index) = response
            .interact_pointer_pos()
            .and_then(|p| at(tab, origin, p))
    {
        tab.text_select = Some(text_select::Selection {
            head: index,
            ..selection
        });
    }
}

pub fn overlay(painter: &egui::Painter, tab: &Tab, origin: Pos2) {
    let Some(selection) = tab.text_select else {
        return;
    };
    let Some((_, tf, data)) = tab.text_layout() else {
        return;
    };
    let (w, h) = geom::size(&data);
    let rects = text_select::range_rects(&data, selection.anchor, selection.head)
        .unwrap_or_else(|| vec![Rect::new(0.0, 0.0, w, h)]);
    for rect in rects {
        let quad = [
            (rect.x0, rect.y0),
            (rect.x1, rect.y0),
            (rect.x1, rect.y1),
            (rect.x0, rect.y1),
        ]
        .map(|(x, y)| {
            let p = tf * Point::new(x, y);
            tab.view
                .to_screen(origin, Pos2::new(p.x as f32, p.y as f32))
        });
        painter.add(egui::Shape::convex_polygon(
            quad.to_vec(),
            theme::ACCENT.gamma_multiply(0.25),
            Stroke::NONE,
        ));
    }
}
