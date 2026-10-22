//! Changed areas of a page: boxes around clusters of changed pixels.
use crate::Region;

const CELL: usize = 16;
const PAD: usize = 24;
const NOISE_PIXELS: u64 = 64;
/// Most areas kept per screen.
pub const MAX_REGIONS: usize = 12;

#[derive(Clone, Copy)]
struct Boxed {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    count: u64,
}

impl Boxed {
    fn padded(self, width: usize, height: usize) -> Self {
        Self {
            x0: self.x0.saturating_sub(PAD),
            y0: self.y0.saturating_sub(PAD),
            x1: (self.x1 + PAD).min(width),
            y1: (self.y1 + PAD).min(height),
            ..self
        }
    }

    fn touches(self, o: Self) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }

    fn union(self, o: Self) -> Self {
        Self {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
            count: self.count + o.count,
        }
    }
}

/// Finds the changed areas of a `width` x `height` mask (`true` = changed pixel).
/// Areas under 64 changed pixels are dropped as specks, unless no larger area exists.
/// Deterministic: largest first, then top-most, then left-most.
#[must_use]
pub fn find_regions(mask: &[bool], width: usize, height: usize) -> Vec<Region> {
    let mut regions = all_regions(mask, width, height);
    regions.truncate(MAX_REGIONS);
    regions
}

pub(crate) fn all_regions(mask: &[bool], width: usize, height: usize) -> Vec<Region> {
    let Some(len) = width.checked_mul(height) else {
        return Vec::new();
    };
    if width == 0 || height == 0 || mask.len() < len {
        return Vec::new();
    }
    let (gw, gh) = (width.div_ceil(CELL), height.div_ceil(CELL));
    let mut hot = vec![false; gw * gh];
    for (i, _) in mask
        .iter()
        .enumerate()
        .take(width * height)
        .filter(|p| *p.1)
    {
        hot[(i / width / CELL) * gw + (i % width) / CELL] = true;
    }
    let mut seen = vec![false; gw * gh];
    let mut boxes = Vec::new();
    for start in 0..gw * gh {
        if !hot[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let mut cells = Vec::new();
        while let Some(c) = stack.pop() {
            let (cx, cy) = (c % gw, c / gw);
            cells.push((cx, cy));
            let near = [
                (cx > 0).then(|| c - 1),
                (cx + 1 < gw).then(|| c + 1),
                (cy > 0).then(|| c - gw),
                (cy + 1 < gh).then(|| c + gw),
            ];
            for n in near.into_iter().flatten() {
                if hot[n] && !seen[n] {
                    seen[n] = true;
                    stack.push(n);
                }
            }
        }
        boxes.push(tight(mask, width, height, &cells));
    }
    let mut boxes: Vec<Boxed> = boxes.into_iter().map(|b| b.padded(width, height)).collect();
    'merge: loop {
        for i in 0..boxes.len() {
            for j in i + 1..boxes.len() {
                if boxes[i].touches(boxes[j]) {
                    boxes[i] = boxes[i].union(boxes[j]);
                    boxes.remove(j);
                    continue 'merge;
                }
            }
        }
        break;
    }
    // Specks are noise only beside a real change. When they are all that changed (a nudged
    // icon, one character), they are the change and must not vanish.
    if boxes.iter().any(|b| b.count >= NOISE_PIXELS) {
        boxes.retain(|b| b.count >= NOISE_PIXELS);
    }
    boxes.sort_by_key(|b| (std::cmp::Reverse(b.count), b.y0, b.x0));
    let to = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    boxes
        .into_iter()
        .map(|b| Region {
            x: to(b.x0),
            y: to(b.y0),
            width: to(b.x1 - b.x0),
            height: to(b.y1 - b.y0),
            pixels_changed: b.count,
            old_png: None,
            new_png: None,
            image_width: None,
            image_height: None,
            nodes: Vec::new(),
            shared: None,
        })
        .collect()
}

/// Exact pixel bounds and count of the changed pixels inside one component's cells.
fn tight(mask: &[bool], width: usize, height: usize, cells: &[(usize, usize)]) -> Boxed {
    let mut b = Boxed {
        x0: width,
        y0: height,
        x1: 0,
        y1: 0,
        count: 0,
    };
    for &(cx, cy) in cells {
        for y in cy * CELL..((cy + 1) * CELL).min(height) {
            for x in cx * CELL..((cx + 1) * CELL).min(width) {
                if mask[y * width + x] {
                    b = Boxed {
                        x0: b.x0.min(x),
                        y0: b.y0.min(y),
                        x1: b.x1.max(x + 1),
                        y1: b.y1.max(y + 1),
                        count: b.count + 1,
                    };
                }
            }
        }
    }
    b
}

/// One region covering every given region, or none when there are none.
pub(crate) fn union(regions: Vec<Region>) -> Vec<Region> {
    let Some(first) = regions.first().cloned() else {
        return regions;
    };
    let (x1, y1) = regions.iter().fold((0, 0), |(x, y), r| {
        (x.max(r.x + r.width), y.max(r.y + r.height))
    });
    let x0 = regions.iter().map(|r| r.x).min().unwrap_or(first.x);
    let y0 = regions.iter().map(|r| r.y).min().unwrap_or(first.y);
    vec![Region {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
        pixels_changed: regions.iter().map(|r| r.pixels_changed).sum(),
        ..first
    }]
}
