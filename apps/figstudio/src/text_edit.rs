//! Editing a text layer in place: typing (and IME commits), deleting, the caret keys, select
//! all and the clipboard, as `SpliceText` edits on the layer. A typing run is one undo step.
//! Text inside an instance stays read-only (its characters come from the component).

use egui::{Event, Key};
use figstudio_fig::Guid;
use figstudio_fig::edit::EditOp;

use crate::tab::Tab;
use crate::text_select::{self, Selection};

/// The layer and selection being edited: a plain text layer in text mode.
pub(crate) fn editing(tab: &Tab) -> Option<Selection> {
    let sel = tab.text_select?;
    let editable = tab.inner.is_none()
        && tab.selection == Some(sel.node)
        && tab.doc.node(sel.node).is_some_and(|n| n.kind == "TEXT");
    editable.then_some(sel)
}

/// Takes this frame's text keys before the app's shortcuts see them (Backspace would delete
/// the layer, ⌘V would paste layers). Returns whether text was being edited.
pub(crate) fn keys(ctx: &egui::Context, tab: &mut Tab) -> bool {
    if editing(tab).is_none() || ctx.egui_wants_keyboard_input() {
        return false;
    }
    let taken: Vec<Event> = ctx.input_mut(|i| {
        let mut taken = Vec::new();
        i.events.retain(|e| {
            let ours = match e {
                Event::Text(_) | Event::Paste(_) | Event::Copy | Event::Cut => true,
                Event::Ime(egui::ImeEvent::Commit(_)) => true,
                // Every plain key (letters would pick tools); with ⌘/Ctrl only the text's own.
                Event::Key { key, modifiers, .. } => {
                    !modifiers.command
                        || matches!(
                            key,
                            Key::A
                                | Key::ArrowLeft
                                | Key::ArrowRight
                                | Key::ArrowUp
                                | Key::ArrowDown
                        )
                }
                _ => false,
            };
            if ours {
                taken.push(e.clone());
            }
            !ours
        });
        taken
    });
    for e in taken {
        apply(ctx, tab, &e);
    }
    true
}

fn apply(ctx: &egui::Context, tab: &mut Tab, e: &Event) {
    let Some(sel) = editing(tab) else { return };
    let Some((_, _, data)) = tab.text_layout() else {
        return;
    };
    let text = text_select::characters(&data).to_string();
    let (lo, hi) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
    let set = |tab: &mut Tab, anchor: usize, head: usize| {
        tab.text_select = Some(Selection {
            anchor,
            head,
            ..sel
        });
    };
    match e {
        Event::Text(t) | Event::Paste(t) | Event::Ime(egui::ImeEvent::Commit(t)) => {
            replace(tab, sel.node, &text, lo, hi, t);
        }
        Event::Copy | Event::Cut => {
            if lo < hi {
                ctx.copy_text(text_select::range_text(&text, lo, hi).to_string());
                if matches!(e, Event::Cut) {
                    replace(tab, sel.node, &text, lo, hi, "");
                }
            }
        }
        Event::Key {
            key,
            pressed: true,
            modifiers: m,
            ..
        } => {
            let prev = |i: usize| text[..i].char_indices().next_back().map_or(0, |(p, _)| p);
            let next = |i: usize| text[i..].chars().next().map_or(i, |c| i + c.len_utf8());
            if !matches!(key, Key::Backspace | Key::Delete | Key::Enter) {
                tab.history.seal_coalesce();
            }
            match key {
                Key::Escape => tab.text_select = None,
                Key::Enter => replace(tab, sel.node, &text, lo, hi, "\n"),
                Key::Backspace if lo < hi => replace(tab, sel.node, &text, lo, hi, ""),
                Key::Backspace if lo > 0 => replace(tab, sel.node, &text, prev(lo), lo, ""),
                Key::Delete if lo < hi => replace(tab, sel.node, &text, lo, hi, ""),
                Key::Delete if hi < text.len() => replace(tab, sel.node, &text, hi, next(hi), ""),
                Key::A if m.command => set(tab, 0, text.len()),
                Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::ArrowDown
                | Key::Home
                | Key::End => {
                    let to = match key {
                        // Collapsing a range goes to its edge.
                        Key::ArrowLeft if lo < hi && !m.shift => lo,
                        Key::ArrowRight if lo < hi && !m.shift => hi,
                        Key::ArrowLeft if m.command => {
                            text_select::line_edge(&data, sel.head, false)
                        }
                        Key::ArrowRight if m.command => {
                            text_select::line_edge(&data, sel.head, true)
                        }
                        Key::Home => text_select::line_edge(&data, sel.head, false),
                        Key::End => text_select::line_edge(&data, sel.head, true),
                        Key::ArrowLeft => prev(sel.head),
                        Key::ArrowRight => next(sel.head),
                        _ => vertical(&data, &text, sel.head, *key == Key::ArrowDown),
                    };
                    if m.shift {
                        set(tab, sel.anchor, to);
                    } else {
                        set(tab, to, to);
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// The caret one line up or down at the same x; past the first or last line, the text's
/// start or end.
fn vertical(data: &figstudio_fig::Val, text: &str, head: usize, down: bool) -> usize {
    let Some((x, top, bottom)) = text_select::caret(data, head) else {
        return head;
    };
    let h = bottom - top;
    let y = if down {
        bottom + h * 0.5
    } else {
        top - h * 0.5
    };
    match text_select::char_at(data, x, y) {
        Some((i, _)) if i != head => i,
        _ if down => text.len(),
        _ => 0,
    }
}

/// Replaces bytes `lo..hi` of `text` with `insert` and puts the caret after it.
fn replace(tab: &mut Tab, id: Guid, text: &str, lo: usize, hi: usize, insert: &str) {
    let lo = text.floor_char_boundary(lo.min(text.len()));
    let hi = text.floor_char_boundary(hi.min(text.len())).max(lo);
    if lo == hi && insert.is_empty() {
        return;
    }
    let op = EditOp::SpliceText {
        target: id,
        utf16_start: text[..lo].encode_utf16().count(),
        utf16_delete_len: text[lo..hi].encode_utf16().count(),
        insert: insert.to_string(),
    };
    // A typing run is one undo step; any other gesture seals it.
    if crate::editor::apply_tab_ops(tab, &[op], Some("Edit text"), Some(&format!("text:{id}")))
        .is_ok()
    {
        let caret = lo + insert.len();
        tab.text_select = Some(Selection {
            node: id,
            anchor: caret,
            head: caret,
        });
    }
}

/// The range being edited in UTF-16 units, for styling part of the text from the panel.
pub(crate) fn utf16_range(tab: &Tab) -> Option<(Guid, usize, usize)> {
    let sel = editing(tab)?;
    let (lo, hi) = (sel.anchor.min(sel.head), sel.anchor.max(sel.head));
    if lo == hi {
        return None;
    }
    let text = text_select::characters(&tab.doc.node(sel.node)?.data);
    let at = |i: usize| {
        text[..text.floor_char_boundary(i.min(text.len()))]
            .encode_utf16()
            .count()
    };
    Some((sel.node, at(lo), at(hi)))
}

#[cfg(test)]
#[path = "text_edit_tests.rs"]
mod tests;
