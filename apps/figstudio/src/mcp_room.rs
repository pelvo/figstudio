//! Persist a Web-Crypto-backed capability; storage denial is visible, never a weak fallback.
use crate::mcp_wire::{new_room, relay_urls, room_link, room_token_from_input, valid_room};
const STORAGE_KEY: &str = "figstudio.mcp.room";

pub(super) fn load() -> Result<String, String> {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    if let Some(room) = storage
        .as_ref()
        .and_then(|s| s.get_item(STORAGE_KEY).ok().flatten())
        .filter(|room| valid_room(room))
    {
        return Ok(room);
    }
    regenerate()
}

/// Store the room for the next visit; the caller shows any failure.
pub(super) fn save(room: &str) -> Result<(), String> {
    storage()?
        .set_item(STORAGE_KEY, room)
        .map_err(|_| "Could not save the MCP room token".to_owned())
}

pub(super) fn regenerate() -> Result<String, String> {
    let room = new_room().map_err(|_| "Could not generate a secure MCP room token".to_owned())?;
    save(&room)?;
    Ok(room)
}

/// Join an existing room named by a token, MCP URL or room link; invalid input changes
/// nothing (the error names the expected forms, never the input, which may hold a secret).
pub(super) fn join(input: &str) -> Result<String, String> {
    let room = room_token_from_input(input)
        .ok_or_else(|| "Not a room token, MCP URL or room link".to_owned())?;
    save(&room)?;
    Ok(room)
}

pub(super) fn urls(room: &str) -> Option<(String, String)> {
    let location = web_sys::window()?.location();
    relay_urls(&location.protocol().ok()?, &location.host().ok()?, room)
}

/// `https://<host>/#room=…`, the link that joins this room on another browser.
pub(super) fn link(room: &str) -> Option<String> {
    let location = web_sys::window()?.location();
    room_link(&location.protocol().ok()?, &location.host().ok()?, room)
}

/// Replace the fragment in the address bar and the history entry (a `#room=` key that
/// was just consumed), so the capability is not left behind in a copied URL.
/// `replaceState` fires no `hashchange`.
pub(super) fn replace_fragment(fragment: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let location = window.location();
    let url = format!(
        "{}{}{}",
        location.pathname().unwrap_or_default(),
        location.search().unwrap_or_default(),
        fragment
    );
    if let Ok(history) = window.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url));
    }
}

fn storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| "Local storage is unavailable; MCP room was not saved".to_owned())
}
