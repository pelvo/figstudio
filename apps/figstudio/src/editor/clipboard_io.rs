//! Two-way Figma HTML clipboard exchange (`<!--(figmeta)-->` + `<!--(figma)-->`) plus plain-text
//! paste fallback when `⌘V` / `Ctrl+V` arrives via `egui::Event::Paste`.

use std::sync::Arc;

use figstudio_fig::clipboard::encode_clipboard_html;
use figstudio_fig::edit::{EditOp, execute_paste_html};

use crate::tab::Tab;

use super::apply_tab_ops;

/// Copies the current selection as Figma `text/html` clipboard markup (plus plain-text fallback).
pub fn copy_selection_for_figma(ctx: &egui::Context, tab: &Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to copy for Figma".to_string())?;
    let html = encode_clipboard_html(&tab.doc, &[sel], 1, tab.doc_key.as_deref())
        .map_err(|e| e.to_string())?;
    let name = tab
        .doc
        .node(sel)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    write_system_html_clipboard(ctx, &html, &name)?;
    Ok(format!("Copied “{name}” for Figma"))
}

/// Pastes Figma HTML from `event_paste` or the system clipboard; if neither holds Figma layer
/// markup, falls back to pasting non-empty plain text (updating a selected `TEXT` layer or
/// creating a new `TEXT` layer at the centre of the viewport).
pub fn paste_from_clipboard_or_text(
    tab: &mut Tab,
    event_paste: Option<&str>,
) -> Result<String, String> {
    if let Some(raw) = event_paste
        && raw.contains("(figma)")
    {
        return paste_figma_markup(tab, raw);
    }
    if let Ok(sys_html) = read_system_html_clipboard() {
        return paste_figma_markup(tab, &sys_html);
    }
    let plain_candidate = event_paste
        .map(str::to_owned)
        .or_else(|| read_system_plain_text().ok())
        .filter(|s| !s.trim().is_empty());
    if let Some(text) = plain_candidate {
        return paste_plain_text_layer(tab, &text);
    }
    Err("Clipboard does not contain Figma layer markup or text".into())
}

fn paste_figma_markup(tab: &mut Tab, html: &str) -> Result<String, String> {
    let target_parent = tab
        .selection
        .filter(|&id| {
            tab.doc.node(id).is_some_and(|n| {
                matches!(n.kind.as_str(), "FRAME" | "SECTION" | "GROUP" | "CANVAS")
            })
        })
        .unwrap_or(tab.page);
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let (roots, _) =
        execute_paste_html(doc_mut, &mut tab.history, html, target_parent, (24.0, 24.0))
            .map_err(|e| e.to_string())?;
    tab.sync_unsaved();
    tab.motion_preview = None;
    if let Some(&first) = roots.first() {
        tab.select(Some(first));
    }
    Ok(format!("Pasted {} layer(s)", roots.len()))
}

fn paste_plain_text_layer(tab: &mut Tab, text: &str) -> Result<String, String> {
    if let Some(sel) = tab.selection
        && tab.doc.node(sel).is_some_and(|n| n.kind == "TEXT")
    {
        apply_tab_ops(
            tab,
            &[EditOp::SetText {
                target: sel,
                characters: text.to_owned(),
            }],
            Some("Paste text"),
            None,
        )?;
        return Ok("Pasted text into selected layer".into());
    }
    let (cx, cy) = tab
        .canvas_rect
        .map(|r| {
            let p = tab.view.to_page(r.min, r.center());
            (f64::from(p.x).round(), f64::from(p.y).round())
        })
        .unwrap_or((120.0, 120.0));
    let ids = apply_tab_ops(
        tab,
        &[EditOp::CreateText {
            id: None,
            parent: tab.page,
            x: cx,
            y: cy,
            characters: text.to_owned(),
            font_family: "Inter".into(),
            font_style: "Regular".into(),
            font_size: 20.0,
            fill_hex: "#111827".into(),
        }],
        Some("Paste text layer"),
        None,
    )?;
    if let Some(&first) = ids.first() {
        tab.select(Some(first));
    }
    Ok("Pasted text layer".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_system_html_clipboard(
    _ctx: &egui::Context,
    html: &str,
    plain: &str,
) -> Result<(), String> {
    use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
    let cb = ClipboardContext::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    cb.set(vec![
        ClipboardContent::Html(html.to_owned()),
        ClipboardContent::Text(plain.to_owned()),
    ])
    .map_err(|e| format!("Could not write Figma HTML clipboard: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn write_system_html_clipboard(
    ctx: &egui::Context,
    html: &str,
    _plain: &str,
) -> Result<(), String> {
    ctx.copy_text(html.to_owned());
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_system_html_clipboard() -> Result<String, String> {
    if cfg!(test) {
        return Err("System clipboard skipped in unit tests".into());
    }
    use clipboard_rs::{Clipboard, ClipboardContext};
    let cb = ClipboardContext::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    if let Ok(h) = cb.get_html()
        && h.contains("(figma)")
    {
        return Ok(h);
    }
    if let Ok(t) = cb.get_text()
        && t.contains("(figma)")
    {
        return Ok(t);
    }
    Err("Clipboard does not contain Figma layer data".into())
}

#[cfg(target_arch = "wasm32")]
fn read_system_html_clipboard() -> Result<String, String> {
    Err("Use browser paste event or MCP paste_from_figma".into())
}

#[cfg(not(target_arch = "wasm32"))]
fn read_system_plain_text() -> Result<String, String> {
    if cfg!(test) {
        return Err("System clipboard skipped in unit tests".into());
    }
    use clipboard_rs::{Clipboard, ClipboardContext};
    let cb = ClipboardContext::new().map_err(|e| format!("Clipboard unavailable: {e}"))?;
    cb.get_text().map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
fn read_system_plain_text() -> Result<String, String> {
    Err("Plain text arrives via Event::Paste in web mode".into())
}
