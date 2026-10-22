//! Persist the executor capability (`figstudio.mcp.executor.v2`); storage denial is
//! visible, never a weak fallback. The caller room is always derived, never stored.
use crate::mcp_pair::{self, ExecutorPair, Load, pair_from_input, pair_link};
use crate::mcp_wire::{
    CloudRelay, RELAY_PROBE_HEADER, RELAY_PROBE_PATH, RELAY_PROBE_TIMEOUT_MS, relay_urls,
};

const STORAGE_KEY: &str = "figstudio.mcp.executor.v2";
const LEGACY_KEY: &str = "figstudio.mcp.room";
const DEVICE_ID_KEY: &str = "figstudio.mcp.device.v1";
const DEVICE_LABEL_KEY: &str = "figstudio.mcp.device_label.v1";

pub(super) struct DeviceLoad {
    pub id: Option<String>,
    pub label: String,
    pub instance: Option<String>,
    pub notices: Vec<String>,
}

fn default_device_label() -> String {
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();
    let os = if ua.contains("Mac") {
        "Mac"
    } else if ua.contains("Win") {
        "Windows"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        "Device"
    };
    let browser = if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome/") {
        "Chrome"
    } else if ua.contains("Firefox/") {
        "Firefox"
    } else if ua.contains("Safari/") {
        "Safari"
    } else {
        "Browser"
    };
    format!("{os} · {browser}")
}

pub(super) fn load_device() -> DeviceLoad {
    let store = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    let mut notices = Vec::new();
    let existing_id = store
        .as_ref()
        .and_then(|s| s.get_item(DEVICE_ID_KEY).ok().flatten())
        .filter(|s| mcp_pair::valid_device_id(s));
    let id = match existing_id {
        Some(saved) => Some(saved),
        None => match mcp_pair::random_hex_16() {
            Ok(fresh) => {
                if let Some(s) = &store {
                    let _ = s.set_item(DEVICE_ID_KEY, &fresh);
                }
                Some(fresh)
            }
            Err(_) => {
                notices.push("Could not generate a secure MCP device identity".to_owned());
                None
            }
        },
    };
    let label = raw_stored_device_label().unwrap_or_else(default_device_label);
    let instance = match mcp_pair::random_hex_16() {
        Ok(fresh) => Some(fresh[..16].to_owned()),
        Err(_) => {
            if notices.is_empty() {
                notices.push("Could not generate a secure MCP device identity".to_owned());
            }
            None
        }
    };
    DeviceLoad {
        id,
        label,
        instance,
        notices,
    }
}

pub(super) fn save_device_label(label: &str) {
    if let Ok(s) = storage() {
        let bounded: String = label.chars().take(64).collect();
        let _ = s.set_item(DEVICE_LABEL_KEY, &bounded);
    }
}

pub(super) fn raw_stored_device_label() -> Option<String> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(DEVICE_LABEL_KEY).ok().flatten())
}

pub(super) fn current_device_label() -> String {
    let stored = raw_stored_device_label();
    mcp_pair::resolve_device_label(stored.as_deref(), &default_device_label())
}

pub(super) fn document_focused() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.has_focus().ok())
        .unwrap_or(true)
}

/// Retry a lost relay at once when the page is shown again or the browser is back
/// online, instead of waiting out the retry delay: sets `hurry` and asks for a frame.
pub(super) fn watch_return(hurry: std::rc::Rc<std::cell::Cell<bool>>, ctx: &egui::Context) {
    use wasm_bindgen::{JsCast, closure::Closure};
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let nudge = move || {
        hurry.set(true);
    };
    let (shown, ctx_shown) = (nudge.clone(), ctx.clone());
    let on_visible = Closure::<dyn FnMut()>::new(move || {
        if !crate::web::page_hidden() {
            shown();
            ctx_shown.request_repaint();
        }
    });
    let ctx = ctx.clone();
    let on_online = Closure::<dyn FnMut()>::new(move || {
        nudge();
        ctx.request_repaint();
    });
    let _ = document
        .add_event_listener_with_callback("visibilitychange", on_visible.as_ref().unchecked_ref());
    let _ = window.add_event_listener_with_callback("online", on_online.as_ref().unchecked_ref());
    // The listeners live as long as the page does.
    on_visible.forget();
    on_online.forget();
}

/// The pair this visit uses, plus one-time notices for the host to toast.
pub(super) struct Loaded {
    pub pair: Option<ExecutorPair>,
    pub notices: Vec<String>,
}

/// Read storage, apply the load plan, and migrate a legacy caller-only room if one
/// lingers. The order is the contract: a new executor is saved before the legacy key
/// is deleted, and a storage failure adopts and deletes nothing.
pub(super) fn load() -> Loaded {
    let storage = web_sys::window().and_then(|w| w.local_storage().ok().flatten());
    let item = |key: &str| {
        storage
            .as_ref()
            .and_then(|s| s.get_item(key).ok().flatten())
    };
    let legacy = item(LEGACY_KEY);
    match mcp_pair::plan(item(STORAGE_KEY), legacy) {
        Load::Use { pair, legacy } => {
            // A leftover legacy key is dead data; remove it now, retry next visit on failure.
            if legacy {
                let _ = storage.as_ref().map(|s| s.remove_item(LEGACY_KEY));
            }
            Loaded {
                pair: Some(pair),
                notices: Vec::new(),
            }
        }
        // A fresh install mints quietly; a legacy caller-only room is migrated by minting
        // a NEW pair (never adopted as the executor); a corrupt v2 value is replaced.
        // In every case a storage failure adopts and deletes nothing.
        Load::Fresh => mint(&storage),
        Load::Upgrade => migrated(
            mint(&storage),
            &storage,
            "MCP room security upgraded; the old room link no longer works — copy the new MCP URL or room link once",
        ),
        Load::Corrupt => migrated(
            mint(&storage),
            &storage,
            "The saved MCP executor token was invalid; a new one was generated — copy the new MCP URL once",
        ),
    }
}

/// A minted replacement for stored credentials: once the new executor is safely saved,
/// delete the legacy key and say once what the user must re-copy.
fn migrated(mut loaded: Loaded, storage: &Option<web_sys::Storage>, notice: &str) -> Loaded {
    if loaded.pair.is_some() {
        let _ = storage.as_ref().map(|s| s.remove_item(LEGACY_KEY));
        loaded.notices.push(notice.to_owned());
    }
    loaded
}

/// Mint and persist a fresh pair. On any failure nothing is adopted or deleted.
fn mint(storage: &Option<web_sys::Storage>) -> Loaded {
    let pair = match ExecutorPair::generate() {
        Ok(pair) => pair,
        Err(_) => {
            return Loaded {
                pair: None,
                notices: vec!["Could not generate a secure MCP executor token".to_owned()],
            };
        }
    };
    match storage
        .as_ref()
        .ok_or_else(|| "Local storage is unavailable; MCP executor was not saved".to_owned())
        .and_then(|s| {
            s.set_item(STORAGE_KEY, pair.executor())
                .map_err(|_| "Could not save the MCP executor token".to_owned())
        }) {
        Ok(()) => Loaded {
            pair: Some(pair),
            notices: Vec::new(),
        },
        Err(error) => Loaded {
            pair: None,
            notices: vec![error],
        },
    }
}

/// Store the executor for the next visit; the caller shows any failure.
pub(super) fn save(pair: &ExecutorPair) -> Result<(), String> {
    storage()?
        .set_item(STORAGE_KEY, pair.executor())
        .map_err(|_| "Could not save the MCP executor token".to_owned())
}

/// Adopt an existing room by its executor: typed here, so consent is already given.
/// Invalid input changes nothing; the error names the accepted forms, never the input,
/// which may hold a secret.
pub(super) fn join(input: &str) -> Result<ExecutorPair, String> {
    let pair = pair_from_input(input).ok_or_else(|| {
        "Only an executor token (ex_…) or a complete room link can join; an MCP URL or \
         room token alone cannot"
            .to_owned()
    })?;
    save(&pair)?;
    Ok(pair)
}

/// A fresh pair, saved before the caller disconnects anything.
pub(super) fn regenerate() -> Result<ExecutorPair, String> {
    let pair = ExecutorPair::generate()
        .map_err(|_| "Could not generate a secure MCP executor token".to_owned())?;
    save(&pair)?;
    Ok(pair)
}

pub(super) fn urls(room: &str) -> Option<(String, String)> {
    let location = web_sys::window()?.location();
    relay_urls(&location.protocol().ok()?, &location.host().ok()?, room)
}

/// Ask this origin whether it runs the cloud relay Worker (`GET /mcp/`). Only the status
/// and one header are read; the request carries no room or executor.
pub(super) async fn probe() -> CloudRelay {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        return CloudRelay::from_probe(None, None);
    };
    // Past the deadline the race settles with `undefined`, which is not a Response: the
    // same failed request as a thrown fetch. The timer is always cleared, and a request
    // the deadline beat is aborted, so Connects on a black-holed server never pile up
    // open GETs.
    let abort = web_sys::AbortController::new().ok();
    let init = web_sys::RequestInit::new();
    init.set_signal(abort.as_ref().map(|abort| abort.signal()).as_ref());
    let mut timer = None;
    let deadline = js_sys::Promise::new(&mut |resolve, _| {
        timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                resolve.unchecked_ref(),
                RELAY_PROBE_TIMEOUT_MS,
            )
            .ok();
    });
    let fetch = window.fetch_with_str_and_init(RELAY_PROBE_PATH, &init);
    let race = js_sys::Promise::race(&js_sys::Array::of2(&fetch, &deadline));
    let answer = wasm_bindgen_futures::JsFuture::from(race)
        .await
        .ok()
        .and_then(|value| value.dyn_into::<web_sys::Response>().ok());
    if let Some(timer) = timer {
        window.clear_timeout_with_handle(timer);
    }
    let Some(response) = answer else {
        if let Some(abort) = &abort {
            abort.abort();
        }
        return CloudRelay::from_probe(None, None);
    };
    let header = response.headers().get(RELAY_PROBE_HEADER).ok().flatten();
    CloudRelay::from_probe(Some(response.status()), header.as_deref())
}

/// `https://<host>/#room=<derived>&exec=<executor>`, the link that joins this room —
/// with executor authority — in another browser.
pub(super) fn link(pair: &ExecutorPair) -> Option<String> {
    let location = web_sys::window()?.location();
    pair_link(&location.protocol().ok()?, &location.host().ok()?, pair)
}

/// Replace the fragment in the address bar and the history entry (the `#room=`/`#exec=`
/// keys that were just consumed), so the capability is not left behind in a copied URL.
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
        .ok_or_else(|| "Local storage is unavailable; MCP executor was not saved".to_owned())
}
