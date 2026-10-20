//! Read-only text ranges. Indices are UTF-8 byte offsets, never glyph ordinals.

use figstudio_fig::{Guid, Val};
use figstudio_render::kurbo::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub node: Guid,
    pub anchor: usize,
    pub head: usize,
}

#[derive(Clone, Copy, Debug)]
struct Glyph {
    index: usize,
    ch: char,
    rect: Rect,
    baseline: f64,
}

pub fn characters(data: &Val) -> &str {
    data.path(&["textData", "characters"])
        .and_then(Val::as_str)
        .unwrap_or("")
}

// shortcut: stored glyphs have positions, not advances. Use the next x on the same
// line, else fontSize * 0.6; replace with shaping clusters when those are available.
fn glyphs(data: &Val) -> Vec<Glyph> {
    let Some(stored) = data.path(&["derivedTextData", "glyphs"]) else {
        return Vec::new();
    };
    let stored = stored.items();
    characters(data)
        .char_indices()
        .zip(stored)
        .filter_map(|((index, ch), g)| {
            let p = g.get("position")?;
            let x = f64::from(p.num("x")?);
            let y = f64::from(p.num("y")?);
            let size = f64::from(g.num("fontSize")?);
            if !x.is_finite() || !y.is_finite() || !size.is_finite() || size <= 0.0 {
                return None;
            }
            Some(Glyph {
                index,
                ch,
                rect: Rect::new(x, y - size, x + size * 0.6, y + size * 0.2),
                baseline: y,
            })
        })
        .collect()
}

fn laid_out(data: &Val) -> Vec<Glyph> {
    let mut glyphs = glyphs(data);
    for i in 0..glyphs.len().saturating_sub(1) {
        let next = glyphs[i + 1];
        let g = &mut glyphs[i];
        if (g.baseline - next.baseline).abs() < 0.01 && next.rect.x0 > g.rect.x0 {
            g.rect.x1 = next.rect.x0;
        }
    }
    glyphs
}

/// Hit a stored line; points past either end clamp to that line's text boundaries.
/// `data` is the TEXT node's data, including its characters and derived glyphs.
pub fn char_at(data: &Val, x: f64, y: f64) -> Option<(usize, char)> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let glyphs = laid_out(data);
    let line = glyphs
        .iter()
        .filter(|g| y >= g.rect.y0 && y <= g.rect.y1)
        .min_by(|a, b| (a.baseline - y).abs().total_cmp(&(b.baseline - y).abs()))?
        .baseline;
    let mut last = None;
    for g in glyphs.iter().filter(|g| (g.baseline - line).abs() < 0.01) {
        if x < g.rect.x1 {
            return Some((g.index, g.ch));
        }
        last = Some(g);
    }
    let last = last?;
    Some((last.index.saturating_add(last.ch.len_utf8()), '\0'))
}

pub fn range_text(characters: &str, a: usize, b: usize) -> &str {
    let boundary = |i: usize| characters.floor_char_boundary(i.min(characters.len()));
    &characters[boundary(a.min(b))..boundary(a.max(b))]
}

/// One node-local rectangle per covered line. No layout means a whole-node overlay.
pub fn range_rects(data: &Val, a: usize, b: usize) -> Option<Vec<Rect>> {
    let glyphs = laid_out(data);
    if glyphs.is_empty() {
        return None;
    }
    let (a, b) = (a.min(b), a.max(b));
    let mut lines: Vec<(f64, Rect)> = Vec::new();
    for g in glyphs.iter().filter(|g| g.index >= a && g.index < b) {
        if let Some((_, rect)) = lines
            .iter_mut()
            .find(|(y, _)| (*y - g.baseline).abs() < 0.01)
        {
            *rect = rect.union(g.rect);
        } else {
            lines.push((g.baseline, g.rect));
        }
    }
    Some(lines.into_iter().map(|(_, rect)| rect).collect())
}

#[cfg(test)]
#[path = "text_select_tests.rs"]
mod tests;
