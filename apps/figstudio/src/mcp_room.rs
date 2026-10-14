//! Persist a Web-Crypto-backed capability; storage denial is visible, never a weak fallback.
use crate::mcp_wire::{new_room, relay_urls, valid_room};
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

pub(super) fn regenerate() -> Result<String, String> {
    let room = new_room().map_err(|_| "Could not generate a secure MCP room token".to_owned())?;
    let storage = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| "Local storage is unavailable; MCP room was not saved".to_owned())?;
    storage
        .set_item(STORAGE_KEY, &room)
        .map_err(|_| "Could not save the MCP room token".to_owned())?;
    Ok(room)
}

pub(super) fn urls(room: &str) -> Option<(String, String)> {
    let location = web_sys::window()?.location();
    relay_urls(&location.protocol().ok()?, &location.host().ok()?, room)
}
