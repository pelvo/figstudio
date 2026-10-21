//! Where the browser keeps the Recents list (`recents.rs`): IndexedDB, through
//! `js/recents.js`. Each entry is the file's name, and where the browser gives file handles
//! (Chrome, Edge) its handle's number, so a card opens the file again. Pictures are PNGs
//! drawn on the thread pool and kept in IndexedDB too. Nothing leaves the browser.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};

use wasm_bindgen::prelude::*;

use crate::recents::{Entry, Id, MAX_ENTRIES, News, ThumbJob, thumb_png};

#[wasm_bindgen(module = "/js/recents.js")]
extern "C" {
    #[wasm_bindgen(js_name = loadList, catch)]
    async fn load_list() -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = saveList, catch)]
    async fn save_list(json: &str) -> Result<JsValue, JsValue>;
    /// Whether this browser gives file handles (the File System Access API).
    #[wasm_bindgen(js_name = keepsHandles)]
    pub(crate) fn keeps_handles() -> bool;
    #[wasm_bindgen(js_name = pickFiles, catch)]
    async fn pick_files_js(start_at: Option<u32>) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = openHandle, catch)]
    async fn open_handle_js(id: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = forgetHandle, catch)]
    async fn forget_handle(id: u32) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = putThumb, catch)]
    async fn put_thumb(key: &str, bytes: &js_sys::Uint8Array) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = getThumb, catch)]
    async fn get_thumb(key: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = deleteThumb, catch)]
    async fn delete_thumb(key: &str) -> Result<JsValue, JsValue>;
    /// Starts keeping the handles of dropped files (once, on the main thread).
    #[wasm_bindgen(js_name = watchDrops)]
    pub(crate) fn watch_drops();
    #[wasm_bindgen(js_name = droppedHandle, catch)]
    async fn dropped_handle_js(name: &str, size: f64, modified: f64) -> Result<JsValue, JsValue>;
}

/// Largest list read (bytes of JSON).
const MAX_BYTES: usize = 1 << 20;
/// Longest file name kept.
const MAX_NAME: usize = 1024;

/// Files picked with the browser's File System Access picker, each with its handle's
/// number; `Ok(None)` when the person cancelled. `start_at`: a handle whose folder the
/// picker starts in.
pub(crate) async fn pick_files(
    start_at: Option<u32>,
) -> Result<Option<Vec<(web_sys::File, u32)>>, String> {
    let picked = pick_files_js(start_at).await.map_err(js_error)?;
    if picked.is_null() || picked.is_undefined() {
        return Ok(None);
    }
    let mut out = Vec::new();
    for item in js_sys::Array::from(&picked).iter() {
        let file = js_sys::Reflect::get(&item, &"file".into()).map_err(js_error)?;
        let handle = js_sys::Reflect::get(&item, &"handle".into()).map_err(js_error)?;
        let (Ok(file), Some(handle)) = (file.dyn_into::<web_sys::File>(), handle.as_f64()) else {
            continue;
        };
        out.push((file, handle as u32));
    }
    Ok(Some(out))
}

/// Why a kept handle did not give its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unavailable {
    /// The file was moved or deleted (or its handle is no longer kept).
    Missing,
    /// The person did not allow reading it.
    Denied,
}

/// The file behind handle `id`, asking for permission to read it if needed (call it from a
/// click: the browser wants one).
pub(crate) async fn open_handle(id: u32) -> Result<web_sys::File, Unavailable> {
    let Ok(answer) = open_handle_js(id).await else {
        return Err(Unavailable::Missing);
    };
    if let Ok(file) = js_sys::Reflect::get(&answer, &"file".into())
        && let Ok(file) = file.dyn_into::<web_sys::File>()
    {
        return Ok(file);
    }
    let error = js_sys::Reflect::get(&answer, &"error".into())
        .ok()
        .and_then(|e| e.as_string());
    Err(match error.as_deref() {
        Some("denied") => Unavailable::Denied,
        _ => Unavailable::Missing,
    })
}

/// The handle number of a dropped file, where the browser gave one.
pub(crate) async fn dropped_handle(file: &web_sys::File) -> Option<u32> {
    dropped_handle_js(&file.name(), file.size(), file.last_modified())
        .await
        .ok()
        .and_then(|v| v.as_f64())
        .map(|n| n as u32)
}

fn js_error(e: JsValue) -> String {
    e.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&e, &"message".into())
                .ok()
                .and_then(|m| m.as_string())
        })
        .unwrap_or_else(|| "the browser refused".into())
}

/// What the background calls answered.
enum Event {
    /// The saved list (`None`: none, or not read).
    Loaded(Option<Vec<Entry>>),
    /// A saved picture (PNG), `None` when there is none.
    Thumb(String, Option<Vec<u8>>),
    /// A picture drawn by job `number` (or why it could not be).
    Painted(String, u64, Result<Vec<u8>, String>),
}

/// The list in IndexedDB, the pictures, and the answers of the background calls.
pub(crate) struct Io {
    /// Whether anything is kept (`false`: the list lives in memory).
    keep: bool,
    /// The saved list was read: saving no longer risks writing over it.
    loaded: bool,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    /// By picture key; `None` for none.
    textures: HashMap<String, Option<egui::TextureHandle>>,
    /// Pictures read or drawn, waiting to become textures.
    ready: HashMap<String, Vec<u8>>,
    /// Pictures being read.
    reading: HashSet<String>,
    /// Each picture key's newest job: an older job, or one for a file removed since, keeps
    /// nothing (as on the desktop).
    wanted: HashMap<String, u64>,
    next_job: u64,
    /// Pictures being drawn on the pool (it cannot wake the window; `busy`).
    painting: usize,
}

impl std::fmt::Debug for Io {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io")
            .field("keep", &self.keep)
            .field("loaded", &self.loaded)
            .finish_non_exhaustive()
    }
}

/// Where the picture of `id` is kept.
fn key(id: &Id) -> String {
    match id.handle {
        Some(h) => format!("h{h}"),
        None => format!("n:{}", id.path.display()),
    }
}

impl Io {
    /// Nothing kept.
    pub fn memory() -> Self {
        let (tx, rx) = channel();
        Self {
            keep: false,
            loaded: true,
            tx,
            rx,
            textures: HashMap::new(),
            ready: HashMap::new(),
            reading: HashSet::new(),
            wanted: HashMap::new(),
            next_job: 0,
            painting: 0,
        }
    }

    /// This browser's list, read in the background (it arrives through `begin_frame`).
    pub fn load() -> (Self, Vec<Entry>) {
        let mut io = Self::memory();
        io.keep = true;
        io.loaded = false;
        let tx = io.tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let list = match load_list().await {
                Ok(json) => json.as_string().map_or(Some(Vec::new()), |j| parse(&j)),
                Err(e) => {
                    tracing::warn!("recent files not read: {}", js_error(e));
                    None
                }
            };
            let _ = tx.send(Event::Loaded(list));
        });
        (io, Vec::new())
    }

    pub fn save(&self, entries: &[Entry]) {
        if !self.keep || !self.loaded {
            return;
        }
        let Ok(json) = serde_json::to_string(entries) else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = save_list(&json).await {
                tracing::warn!("recent files not saved: {}", js_error(e));
            }
        });
    }

    /// `id` left the list: its picture and handle go too.
    pub fn forget(&mut self, id: &Id) {
        let key = key(id);
        self.textures.remove(&key);
        self.ready.remove(&key);
        self.wanted.remove(&key);
        if !self.keep {
            return;
        }
        let handle = id.handle;
        wasm_bindgen_futures::spawn_local(async move {
            let _ = delete_thumb(&key).await;
            if let Some(h) = handle {
                let _ = forget_handle(h).await;
            }
        });
    }

    /// Draws the picture of `id` on the thread pool.
    pub fn paint(&mut self, _ctx: &egui::Context, id: &Id, job: ThumbJob) {
        let key = key(id);
        self.next_job += 1;
        let number = self.next_job;
        self.wanted.insert(key.clone(), number);
        self.painting += 1;
        let tx = self.tx.clone();
        rayon::spawn(move || {
            let png = thumb_png(&job).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Painted(key, number, png));
        });
    }

    /// A picture is being drawn: the window is drawn again shortly to collect it.
    pub fn busy(&self) -> bool {
        self.painting > 0
    }

    /// The texture of `id`'s picture, decoding at most `budget` more this frame.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        id: &Id,
        budget: &mut usize,
    ) -> Option<egui::TextureHandle> {
        let key = key(id);
        if let Some(t) = self.textures.get(&key) {
            return t.clone();
        }
        if self.ready.contains_key(&key) {
            if *budget == 0 {
                ctx.request_repaint();
                return None;
            }
            *budget -= 1;
            let png = self.ready.remove(&key)?;
            let texture = crate::recents::png_texture(ctx, &format!("recent-{key}"), &png);
            self.textures.insert(key, texture.clone());
            return texture;
        }
        if self.keep && self.reading.insert(key.clone()) {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let bytes = get_thumb(&key)
                    .await
                    .ok()
                    .filter(|v| !v.is_null() && !v.is_undefined())
                    .map(|v| js_sys::Uint8Array::new(&v).to_vec());
                let _ = tx.send(Event::Thumb(key, bytes));
                ctx.request_repaint();
            });
        }
        None
    }

    /// Nothing to look at again: a browser file is only tried when it is clicked.
    pub fn changed(&mut self) {}

    /// Takes the background calls' answers.
    pub fn begin_frame(
        &mut self,
        _ctx: &egui::Context,
        _time: f64,
        _appeared: bool,
        _entries: &[Entry],
    ) -> News {
        self.poll()
    }

    /// Takes the background calls' answers (also while the page is not drawn).
    pub fn poll(&mut self) -> News {
        let mut news = News::default();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Loaded(list) => {
                    // Not read (the store failed): this session's list stays in memory and
                    // the saved one is not written over.
                    self.keep = list.is_some();
                    self.loaded = true;
                    news.entries = Some(list.unwrap_or_default());
                }
                Event::Thumb(key, bytes) => {
                    self.reading.remove(&key);
                    match bytes {
                        Some(png) => {
                            self.ready.insert(key, png);
                        }
                        None => {
                            self.textures.insert(key, None);
                        }
                    }
                }
                Event::Painted(key, number, png) => {
                    self.painting = self.painting.saturating_sub(1);
                    let png = match png {
                        Ok(png) => png,
                        Err(e) => {
                            tracing::warn!("no picture for a recent file: {e}");
                            continue;
                        }
                    };
                    if self.wanted.get(&key) != Some(&number) {
                        continue;
                    }
                    self.wanted.remove(&key);
                    self.textures.remove(&key);
                    if self.keep {
                        let (key, bytes) = (key.clone(), js_sys::Uint8Array::from(&png[..]));
                        wasm_bindgen_futures::spawn_local(async move {
                            if let Err(e) = put_thumb(&key, &bytes).await {
                                tracing::warn!("picture not saved: {}", js_error(e));
                            }
                        });
                    }
                    self.ready.insert(key, png);
                }
            }
        }
        news
    }
}

/// The entries of a saved list, skipping damaged ones; `None` when it is not a JSON list.
fn parse(json: &str) -> Option<Vec<Entry>> {
    if json.len() > MAX_BYTES {
        return None;
    }
    let items: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    let mut entries: Vec<Entry> = items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .filter(|e: &Entry| {
            let name = e.path.as_os_str().len();
            name > 0 && name <= MAX_NAME && e.key.as_ref().is_none_or(|k| k.len() <= 64)
        })
        .collect();
    entries.truncate(MAX_ENTRIES);
    Some(entries)
}
