// Browser save bridge for large lazy `.fig` archives (web.rs).
//
// Streams newly encoded headers/canvas + zero-copy `File.slice()` Blob ranges
// into a File System Access writable stream (atomic swap file) when available,
// or a composite `Blob` download otherwise. Untouched archive entries are never
// decompressed or copied into Wasm/JS memory.
//
// No side effects on import: thread-pool workers load this module too, and have
// no window or document of the page.

/** Whether this browser supports the File System Access save picker. */
export function canSavePicker() {
  return typeof globalThis.showSaveFilePicker === "function";
}

/** Prompts for a destination `.fig` file.
 *  Returns `{ status: "handle", handle, name }`, `{ status: "cancelled" }`,
 *  or `{ status: "fallback" }` when the picker is unavailable or blocked. */
export async function pickSaveFile(suggestedName) {
  if (!canSavePicker()) return { status: "fallback" };
  try {
    const handle = await globalThis.showSaveFilePicker({
      suggestedName,
      types: [{ description: "Figma file", accept: { "application/octet-stream": [".fig"] } }],
      excludeAcceptAllOption: false,
    });
    return { status: "handle", handle, name: handle.name || suggestedName };
  } catch (e) {
    if (e && e.name === "AbortError") return { status: "cancelled" };
    if (e && (e.name === "NotAllowedError" || e.name === "SecurityError" || e.name === "TypeError")) {
      return { status: "fallback" };
    }
    throw e;
  }
}

/** Streams `parts` (`Uint8Array` and `Blob` slices) into `handle` via its swap file.
 *  Existing destination bytes stay untouched until `close()` succeeds; any error aborts. */
export async function writeHandleParts(handle, parts) {
  const writable = await handle.createWritable();
  try {
    for (const part of parts) {
      await writable.write(part);
    }
    await writable.close();
  } catch (e) {
    try { await writable.abort(); } catch { /* best-effort swap cleanup */ }
    throw e;
  }
}

/** Downloads a composite `Blob` assembled from `parts` (`Uint8Array` and `Blob` slices). */
export function downloadBlobParts(name, mime, parts) {
  const blob = new Blob(parts, { type: mime });
  const url = URL.createObjectURL(blob);
  const doc = globalThis.document;
  if (!doc) {
    URL.revokeObjectURL(url);
    throw new Error("no document");
  }
  const a = doc.createElement("a");
  a.href = url;
  a.download = name;
  a.style.display = "none";
  if (doc.body) {
    doc.body.appendChild(a);
    a.click();
    a.remove();
  } else {
    a.click();
  }
  // Defer revocation so large composite Blob downloads do not race with URL teardown.
  if (typeof globalThis.setTimeout === "function") {
    globalThis.setTimeout(() => URL.revokeObjectURL(url), 60_000);
  } else {
    URL.revokeObjectURL(url);
  }
}
