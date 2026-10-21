// The browser's Recents store (recents_web.rs): the list, the file handles and the page
// pictures in IndexedDB. Nothing here leaves the browser.
//
// Handles exist where the browser has the File System Access API (Chrome, Edge): a card
// then opens its file again directly. Elsewhere a card is a reminder that opens the picker.
//
// No side effects on import: the thread-pool workers load this module too, and have no
// window or IndexedDB of the page.

const DB = "figstudio-recents";
const LIST = "list";
const HANDLES = "handles";
const THUMBS = "thumbs";

let opened = null;

/** The database, opened (and created) on first use. */
function db() {
  if (!opened) {
    opened = new Promise((resolve, reject) => {
      const req = indexedDB.open(DB, 1);
      req.onupgradeneeded = () => {
        const d = req.result;
        d.createObjectStore(LIST);
        d.createObjectStore(HANDLES, { autoIncrement: true });
        d.createObjectStore(THUMBS);
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    // A failed open is tried again next time, not remembered.
    opened.catch(() => { opened = null; });
  }
  return opened;
}

/** Runs `work(store)` in one transaction on `name`; resolves with its request's result. */
async function run(name, mode, work) {
  const d = await db();
  return new Promise((resolve, reject) => {
    const tx = d.transaction(name, mode);
    const req = work(tx.objectStore(name));
    let result;
    if (req) req.onsuccess = () => { result = req.result; };
    tx.oncomplete = () => resolve(result);
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error);
  });
}

/** The saved list (JSON), or null when there is none. */
export async function loadList() {
  const json = await run(LIST, "readonly", (s) => s.get("entries"));
  return typeof json === "string" ? json : null;
}

export async function saveList(json) {
  await run(LIST, "readwrite", (s) => s.put(json, "entries"));
}

/** Whether this browser gives file handles (and so can open a recent file again). */
export function keepsHandles() {
  return typeof globalThis.showOpenFilePicker === "function";
}

/** The number `handle` is kept under: the same file always gets the same number. */
export async function keepHandle(handle) {
  const d = await db();
  const kept = await new Promise((resolve, reject) => {
    const out = [];
    const req = d.transaction(HANDLES).objectStore(HANDLES).openCursor();
    req.onsuccess = () => {
      const c = req.result;
      if (!c) return resolve(out);
      out.push([c.key, c.value]);
      c.continue();
    };
    req.onerror = () => reject(req.error);
  });
  for (const [key, h] of kept) {
    try {
      if (await h.isSameEntry(handle)) return key;
    } catch { /* a handle whose file is gone compares as different */ }
  }
  return run(HANDLES, "readwrite", (s) => s.add(handle));
}

export async function forgetHandle(id) {
  await run(HANDLES, "readwrite", (s) => s.delete(id));
}

/** Files picked with the File System Access picker, each with its handle's number.
 *  `startAt`: the number of a handle whose folder the picker starts in. `null` when the
 *  person cancelled. */
export async function pickFiles(startAt) {
  const options = {
    multiple: true,
    types: [{ description: "Figma file", accept: { "application/octet-stream": [".fig"] } }],
    excludeAcceptAllOption: false,
  };
  if (startAt != null) {
    const start = await run(HANDLES, "readonly", (s) => s.get(startAt));
    if (start) options.startIn = start;
  }
  let handles;
  try {
    handles = await globalThis.showOpenFilePicker(options);
  } catch (e) {
    if (e && e.name === "AbortError") return null;
    if (options.startIn) {
      // The folder may be gone: open where the browser last was.
      delete options.startIn;
      try { handles = await globalThis.showOpenFilePicker(options); } catch (again) {
        if (again && again.name === "AbortError") return null;
        throw again;
      }
    } else {
      throw e;
    }
  }
  const out = [];
  for (const h of handles) {
    out.push({ file: await h.getFile(), handle: await keepHandle(h) });
  }
  return out;
}

/** The file of handle `id`, asking for read permission if needed (needs the click that
 *  led here). `{ file }`, or `{ error }`: "gone" (no such handle), "denied", "missing". */
export async function openHandle(id) {
  const h = await run(HANDLES, "readonly", (s) => s.get(id));
  if (!h) return { error: "gone" };
  let state = await h.queryPermission({ mode: "read" });
  if (state !== "granted") {
    try { state = await h.requestPermission({ mode: "read" }); } catch { state = "denied"; }
  }
  if (state !== "granted") return { error: "denied" };
  try {
    return { file: await h.getFile() };
  } catch (e) {
    return { error: e && e.name === "NotAllowedError" ? "denied" : "missing" };
  }
}

export async function putThumb(key, bytes) {
  await run(THUMBS, "readwrite", (s) => s.put(bytes, key));
}

/** The picture saved under `key` (PNG bytes), or null. */
export async function getThumb(key) {
  const bytes = await run(THUMBS, "readonly", (s) => s.get(key));
  return bytes instanceof Uint8Array ? bytes : null;
}

export async function deleteThumb(key) {
  await run(THUMBS, "readwrite", (s) => s.delete(key));
}

// Dropped files: the handle is only reachable during the drop event, so a listener on the
// window (capture phase, before the app's) asks for each handle then; the app collects it
// by the file's name, size and date once it reads the `File`. Only the latest drop's files
// are kept, so an older file of the same name, size and date never takes a handle.
let drops = [];

/** Starts keeping the handles of dropped files (the page's main thread, once). */
export function watchDrops() {
  if (!keepsHandles() || typeof window === "undefined") return;
  window.addEventListener("drop", (event) => {
    const items = event.dataTransfer ? [...event.dataTransfer.items] : [];
    drops = [];
    for (const item of items) {
      if (item.kind !== "file" || typeof item.getAsFileSystemHandle !== "function") continue;
      const file = item.getAsFile();
      const handle = item.getAsFileSystemHandle();
      if (!file) continue;
      drops.push({ name: file.name, size: file.size, modified: file.lastModified, handle });
    }
  }, true);
}

/** The handle number of a dropped file, matched by name, size and date; null if none. */
export async function droppedHandle(name, size, modified) {
  const i = drops.findIndex((d) => d.name === name && d.size === size && d.modified === modified);
  if (i < 0) return null;
  const [drop] = drops.splice(i, 1);
  try {
    const h = await drop.handle;
    return h && h.kind === "file" ? await keepHandle(h) : null;
  } catch {
    return null;
  }
}
