//! Screen-space broad phase for canvas chrome. Oversized items stay in one list.
use std::collections::HashMap;

use egui::Rect;

const CELL: f32 = 256.0;
const MAX_CELLS: i64 = 256;

#[derive(Default)]
pub(super) struct Grid<T> {
    cells: HashMap<(i32, i32), Vec<usize>>,
    wide: Vec<usize>,
    items: Vec<(Rect, T)>,
}

fn cells(r: Rect) -> (i32, i32, i32, i32) {
    (
        (r.left() / CELL).floor() as i32,
        (r.top() / CELL).floor() as i32,
        (r.right() / CELL).floor() as i32,
        (r.bottom() / CELL).floor() as i32,
    )
}

fn small((x0, y0, x1, y1): (i32, i32, i32, i32)) -> bool {
    (i64::from(x1) - i64::from(x0) + 1).saturating_mul(i64::from(y1) - i64::from(y0) + 1)
        <= MAX_CELLS
}

impl<T> Grid<T> {
    pub(super) fn insert(&mut self, bounds: Rect, item: T) {
        if !bounds.is_finite() || bounds.is_negative() {
            return;
        }
        let index = self.items.len();
        self.items.push((bounds, item));
        let range = cells(bounds);
        if small(range) {
            let (x0, y0, x1, y1) = range;
            for y in y0..=y1 {
                for x in x0..=x1 {
                    self.cells.entry((x, y)).or_default().push(index);
                }
            }
        } else {
            self.wide.push(index);
        }
    }

    pub(super) fn any(&self, bounds: Rect, mut hit: impl FnMut(&T) -> bool) -> bool {
        let mut overlaps = |index: usize| {
            let (r, item) = &self.items[index];
            bounds.min.x < r.max.x
                && r.min.x < bounds.max.x
                && bounds.min.y < r.max.y
                && r.min.y < bounds.max.y
                && hit(item)
        };
        let range = cells(bounds);
        if !small(range) {
            return (0..self.items.len()).any(overlaps);
        }
        if self.wide.iter().copied().any(&mut overlaps) {
            return true;
        }
        let (x0, y0, x1, y1) = range;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self
                    .cells
                    .get(&(x, y))
                    .is_some_and(|ids| ids.iter().copied().any(&mut overlaps))
                {
                    return true;
                }
            }
        }
        false
    }
}
