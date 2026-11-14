//! Interactive canvas gizmos: 8-handle resize, direct layer drag, and shape/text creation tools.

use egui::{Color32, CornerRadius, CursorIcon, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use figstudio_fig::Guid;
use figstudio_fig::edit::EditOp;

use crate::canvas::Tool;
use crate::tab::Tab;
use crate::theme;

use super::{apply_tab_ops, local_xywh};

/// Which of the 8 resize handles around a selected layer is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::NW | Self::SE => CursorIcon::ResizeNwSe,
            Self::NE | Self::SW => CursorIcon::ResizeNeSw,
            Self::N | Self::S => CursorIcon::ResizeVertical,
            Self::E | Self::W => CursorIcon::ResizeHorizontal,
        }
    }

    pub fn screen_pos(self, r: Rect) -> Pos2 {
        match self {
            Self::NW => r.left_top(),
            Self::N => pos2(r.center().x, r.top()),
            Self::NE => r.right_top(),
            Self::E => pos2(r.right(), r.center().y),
            Self::SE => r.right_bottom(),
            Self::S => pos2(r.center().x, r.bottom()),
            Self::SW => r.left_bottom(),
            Self::W => pos2(r.left(), r.center().y),
        }
    }
}

/// Active interactive gesture on the canvas.
#[derive(Clone, Debug)]
pub enum CanvasDrag {
    Move {
        target: Guid,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        moved: bool,
    },
    Resize {
        target: Guid,
        handle: Handle,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        orig_w: f64,
        orig_h: f64,
    },
    Create {
        tool: Tool,
        start_page: Pos2,
        cur_page: Pos2,
    },
}

/// Handles canvas creation tools, 8-handle resize dragging, and direct selection dragging,
/// returning `Some(cursor)` when an editor gizmo owns pointer interaction this frame.
pub fn canvas_gizmos_and_input(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    tool: &mut Tool,
    panning: bool,
) -> Option<CursorIcon> {
    if panning || tab.preview.is_some() {
        return None;
    }

    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
    let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    // 1. Creation tools (`Frame`, `Section`, `Rectangle`, `Ellipse`, `Line`, `Text`).
    if tool.is_creation() {
        if primary_pressed
            && let Some(pos) = pointer_pos
            && rect.contains(pos)
        {
            let p = tab.view.to_page(rect.min, pos);
            tab.edit_drag = Some(CanvasDrag::Create {
                tool: *tool,
                start_page: p,
                cur_page: p,
            });
        }
        if let Some(CanvasDrag::Create {
            tool: create_tool,
            start_page,
            ref mut cur_page,
        }) = tab.edit_drag
        {
            if let Some(pos) = pointer_pos {
                *cur_page = tab.view.to_page(rect.min, pos);
            }
            let min_p = pos2(start_page.x.min(cur_page.x), start_page.y.min(cur_page.y));
            let max_p = pos2(start_page.x.max(cur_page.x), start_page.y.max(cur_page.y));
            let s0 = tab.view.to_screen(rect.min, min_p);
            let s1 = tab.view.to_screen(rect.min, max_p);
            painter.rect_stroke(
                Rect::from_min_max(s0, s1),
                0.0,
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
            if primary_released {
                let end_p = *cur_page;
                tab.edit_drag = None;
                let w =
                    f64::from((end_p.x - start_page.x).abs()).max(if create_tool == Tool::Text {
                        120.0
                    } else {
                        80.0
                    });
                let h = f64::from((end_p.y - start_page.y).abs())
                    .max(if create_tool == Tool::Line { 0.0 } else { 60.0 });
                let x = f64::from(start_page.x.min(end_p.x));
                let y = f64::from(start_page.y.min(end_p.y));
                let op = match create_tool {
                    Tool::Text => EditOp::CreateText {
                        id: None,
                        parent: tab.page,
                        x,
                        y,
                        characters: "Text".into(),
                        font_family: "Inter".into(),
                        font_style: "Regular".into(),
                        font_size: 20.0,
                        fill_hex: "#111827".into(),
                    },
                    other => EditOp::CreateShape {
                        id: None,
                        kind: other.shape_kind().unwrap_or("FRAME").into(),
                        parent: tab.page,
                        name: None,
                        x,
                        y,
                        width: w,
                        height: h,
                        corner_radius: 0.0,
                        fill_hex: None,
                        count: None,
                        star_inner_scale: None,
                    },
                };
                if let Ok(ids) = apply_tab_ops(tab, &[op], Some("Create layer"), None)
                    && let Some(&new_id) = ids.first()
                {
                    tab.select(Some(new_id));
                }
                *tool = Tool::Select;
            }
        }
        return Some(CursorIcon::Crosshair);
    }

    if *tool != Tool::Select || tab.text_select.is_some() {
        return None;
    }

    // 2. Continue an in-progress Resize or Move drag without walking hit_chain!
    if let Some(drag) = tab.edit_drag.clone() {
        if primary_down && let Some(pos) = pointer_pos {
            let cur_page = tab.view.to_page(rect.min, pos);
            let dx = f64::from(
                cur_page.x
                    - match &drag {
                        CanvasDrag::Move {
                            start_pointer_page, ..
                        } => start_pointer_page.x,
                        CanvasDrag::Resize {
                            start_pointer_page, ..
                        } => start_pointer_page.x,
                        CanvasDrag::Create { start_page, .. } => start_page.x,
                    },
            );
            let dy = f64::from(
                cur_page.y
                    - match &drag {
                        CanvasDrag::Move {
                            start_pointer_page, ..
                        } => start_pointer_page.y,
                        CanvasDrag::Resize {
                            start_pointer_page, ..
                        } => start_pointer_page.y,
                        CanvasDrag::Create { start_page, .. } => start_page.y,
                    },
            );

            match drag {
                CanvasDrag::Move {
                    target,
                    start_pointer_page,
                    orig_x,
                    orig_y,
                    moved,
                } => {
                    if moved || dx.hypot(dy) * f64::from(tab.view.zoom) >= 3.0 {
                        let key = format!("gui:move:{target}");
                        let _ = apply_tab_ops(
                            tab,
                            &[EditOp::Move {
                                target,
                                x: (orig_x + dx).round(),
                                y: (orig_y + dy).round(),
                                relative: false,
                            }],
                            Some("Move layer"),
                            Some(&key),
                        );
                        tab.edit_drag = Some(CanvasDrag::Move {
                            target,
                            start_pointer_page,
                            orig_x,
                            orig_y,
                            moved: true,
                        });
                        return Some(CursorIcon::Grabbing);
                    }
                }
                CanvasDrag::Resize {
                    target,
                    handle,
                    orig_x,
                    orig_y,
                    orig_w,
                    orig_h,
                    ..
                } => {
                    let (mut nx, mut ny, mut nw, mut nh) = (orig_x, orig_y, orig_w, orig_h);
                    match handle {
                        Handle::E | Handle::NE | Handle::SE => nw = (orig_w + dx).max(1.0),
                        Handle::W | Handle::NW | Handle::SW => {
                            nw = (orig_w - dx).max(1.0);
                            nx = orig_x + (orig_w - nw);
                        }
                        _ => {}
                    }
                    match handle {
                        Handle::S | Handle::SE | Handle::SW => nh = (orig_h + dy).max(1.0),
                        Handle::N | Handle::NW | Handle::NE => {
                            nh = (orig_h - dy).max(1.0);
                            ny = orig_y + (orig_h - nh);
                        }
                        _ => {}
                    }
                    let key = format!("gui:resize:{target}");
                    let _ = apply_tab_ops(
                        tab,
                        &[
                            EditOp::Move {
                                target,
                                x: nx.round(),
                                y: ny.round(),
                                relative: false,
                            },
                            EditOp::Resize {
                                target,
                                width: nw.round(),
                                height: nh.round(),
                            },
                        ],
                        Some("Resize layer"),
                        Some(&key),
                    );
                    paint_selection_handles(painter, rect, tab, target);
                    return Some(handle.cursor());
                }
                CanvasDrag::Create { .. } => {}
            }
        }
        if primary_released || !primary_down {
            tab.history.seal_coalesce();
            tab.edit_drag = None;
        }
    }

    // 3. Paint 8 resize handles around the selected node and detect handle hover / press.
    let sel = tab.selection?;
    let node = tab.doc.node(sel)?;
    if matches!(node.kind.as_str(), "DOCUMENT" | "CANVAS") || node.data.flag("locked") == Some(true)
    {
        return None;
    }

    let bounds = paint_selection_handles(painter, rect, tab, sel)?;
    let pos = pointer_pos?;
    if !rect.contains(pos) {
        return None;
    }

    for h in Handle::ALL {
        let hp = h.screen_pos(bounds);
        let hit = Rect::from_center_size(hp, vec2(10.0, 10.0));
        if hit.contains(pos) {
            if primary_pressed {
                let (ox, oy, ow, oh) = local_xywh(&node.data);
                tab.edit_drag = Some(CanvasDrag::Resize {
                    target: sel,
                    handle: h,
                    start_pointer_page: tab.view.to_page(rect.min, pos),
                    orig_x: ox,
                    orig_y: oy,
                    orig_w: ow,
                    orig_h: oh,
                });
            }
            return Some(h.cursor());
        }
    }

    if primary_pressed && bounds.contains(pos) {
        let (ox, oy, _, _) = local_xywh(&node.data);
        tab.edit_drag = Some(CanvasDrag::Move {
            target: sel,
            start_pointer_page: tab.view.to_page(rect.min, pos),
            orig_x: ox,
            orig_y: oy,
            moved: false,
        });
    }
    None
}

fn paint_selection_handles(
    painter: &egui::Painter,
    canvas_rect: Rect,
    tab: &Tab,
    node_id: Guid,
) -> Option<Rect> {
    let b = figstudio_render::node_bounds(&tab.doc, node_id)?;
    let min = tab
        .view
        .to_screen(canvas_rect.min, pos2(b.x0 as f32, b.y0 as f32));
    let max = tab
        .view
        .to_screen(canvas_rect.min, pos2(b.x1 as f32, b.y1 as f32));
    let screen_rect = Rect::from_min_max(min, max);
    for h in Handle::ALL {
        let c = h.screen_pos(screen_rect);
        let hr = Rect::from_center_size(c, vec2(7.0, 7.0));
        painter.rect_filled(hr, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(
            hr,
            CornerRadius::same(1),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
    Some(screen_rect)
}
