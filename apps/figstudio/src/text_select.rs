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
    end: usize,
    ch: char,
    has_advance: bool,
    rect: Rect,
    baseline: f64,
}

pub fn characters(data: &Val) -> &str {
    data.path(&["textData", "characters"])
        .and_then(Val::as_str)
        .unwrap_or("")
}

fn glyphs(data: &Val) -> Vec<Glyph> {
    let Some(stored) = data.path(&["derivedTextData", "glyphs"]) else {
        return Vec::new();
    };
    let stored = stored.items();
    let text = characters(data);
    // firstCharacter counts Unicode scalars, not UTF-16 units or bytes: a public
    // export with a leading emoji has eight glyph starts 0..7, but nine UTF-16 units.
    let chars: Vec<_> = text.char_indices().collect();
    let start = |g: &Val| {
        let ordinal = g.get("firstCharacter")?.as_u32()? as usize;
        chars.get(ordinal).copied()
    };
    let mut starts: Vec<_> = stored
        .iter()
        .filter_map(|g| start(g).map(|(i, _)| i))
        .collect();
    starts.push(text.len());
    // A line break has no glyph; it still ends the cluster before it.
    starts.extend(text.match_indices('\n').map(|(i, _)| i));
    starts.sort_unstable();
    starts.dedup();
    // Like the renderer, this is a glyph count including the inserted ellipsis.
    // That glyph has no firstCharacter and therefore no selectable source cluster.
    let visible = data
        .path(&["derivedTextData", "truncationStartIndex"])
        .and_then(Val::as_u32)
        .map_or(stored.len(), |n| stored.len().min(n as usize));
    stored[..visible]
        .iter()
        .filter_map(|g| {
            let (index, ch) = start(g)?;
            let end = *starts.get(starts.partition_point(|i| *i <= index))?;
            let p = g.get("position")?;
            let x = f64::from(p.num("x")?);
            let y = f64::from(p.num("y")?);
            let size = f64::from(g.num("fontSize")?);
            if !x.is_finite() || !y.is_finite() || !size.is_finite() || size <= 0.0 {
                return None;
            }
            let advance = g
                .num("advance")
                .map(f64::from)
                .filter(|a| a.is_finite() && *a > 0.0);
            Some(Glyph {
                index,
                end,
                ch,
                has_advance: advance.is_some(),
                rect: Rect::new(
                    x,
                    y - size,
                    x + size * advance.unwrap_or(0.6),
                    y + size * 0.2,
                ),
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
        if !g.has_advance && (g.baseline - next.baseline).abs() < 0.01 && next.rect.x0 > g.rect.x0 {
            g.rect.x1 = next.rect.x0;
        }
    }
    glyphs
}

/// Hit the nearest cluster boundary; points past either end clamp to that line's boundaries.
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
    let on_line = glyphs.iter().filter(|g| (g.baseline - line).abs() < 0.01);
    let first = on_line
        .clone()
        .min_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0))?;
    // Kerning can overlap advances: the glyph starting here owns this hit.
    let hit = on_line
        .filter(|g| g.rect.x0 <= x)
        .max_by(|a, b| a.rect.x0.total_cmp(&b.rect.x0))
        .unwrap_or(first);
    if x < (hit.rect.x0 + hit.rect.x1) * 0.5 {
        Some((hit.index, hit.ch))
    } else {
        Some((hit.end, '\0'))
    }
}

/// The caret before byte `index` in node space: `(x, top, bottom)`. Empty text puts it at
/// the start of the box, one font size tall.
pub fn caret(data: &Val, index: usize) -> Option<(f64, f64, f64)> {
    let glyphs = laid_out(data);
    let text = characters(data);
    let Some(last) = glyphs
        .iter()
        .filter(|g| g.index < index)
        .max_by_key(|g| g.index)
    else {
        // Before the first glyph (or no glyphs at all).
        if let Some(g) = glyphs.iter().min_by_key(|g| g.index) {
            return Some((g.rect.x0, g.rect.y0, g.rect.y1));
        }
        let size = f64::from(data.num("fontSize").unwrap_or(12.0));
        return Some((0.0, 0.0, size * 1.2));
    };
    if let Some(g) = glyphs.iter().find(|g| g.index == index) {
        return Some((g.rect.x0, g.rect.y0, g.rect.y1));
    }
    let before = &text[..index.min(text.len())];
    let newlines = before[last.index.min(before.len())..].matches('\n').count();
    if newlines == 0 {
        return Some((last.rect.x1, last.rect.y0, last.rect.y1));
    }
    // After line breaks with no glyph yet: the start of a later line.
    let step = line_step(&glyphs).unwrap_or(last.rect.height());
    let dy = step * newlines as f64;
    Some((0.0, last.rect.y0 + dy, last.rect.y1 + dy))
}

/// The distance between consecutive baselines, when there are two lines.
fn line_step(glyphs: &[Glyph]) -> Option<f64> {
    let first = glyphs.first()?.baseline;
    glyphs
        .iter()
        .map(|g| g.baseline - first)
        .find(|d| *d > 0.01)
}

/// The start or end (`end`) of the line holding byte `index`, as a byte index.
pub fn line_edge(data: &Val, index: usize, end: bool) -> usize {
    let text = characters(data);
    let i = index.min(text.len());
    // Lines break at newlines and where layout wraps: take the laid-out line when known.
    if let Some((_, top, bottom)) = caret(data, i)
        && let Some((hit, _)) = char_at(data, if end { 1e9 } else { -1e9 }, (top + bottom) / 2.0)
    {
        return hit.min(text.len());
    }
    if end {
        text[i..].find('\n').map_or(text.len(), |n| i + n)
    } else {
        text[..i].rfind('\n').map_or(0, |n| n + 1)
    }
}

pub fn range_text(characters: &str, a: usize, b: usize) -> &str {
    let boundary = |i: usize| characters.floor_char_boundary(i.min(characters.len()));
    &characters[boundary(a.min(b))..boundary(a.max(b))]
}

/// One node-local rectangle per covered line. No layout means a whole-node overlay.
pub fn range_rects(data: &Val, a: usize, b: usize) -> Option<Vec<Rect>> {
    let glyphs = laid_out(data);
    if glyphs.is_empty() {
        return data
            .path(&["derivedTextData", "glyphs"])
            .filter(|stored| !stored.items().is_empty())
            .map(|_| Vec::new());
    }
    let (a, b) = (a.min(b), a.max(b));
    let mut lines: Vec<(f64, Rect)> = Vec::new();
    for g in glyphs.iter().filter(|g| a < b && g.end > a && g.index < b) {
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

#[cfg(test)]
#[path = "text_cluster_tests.rs"]
mod cluster_tests;
