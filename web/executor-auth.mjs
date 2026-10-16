// Executor proof for the relay Worker. The browser tab holds a 256-bit executor
// secret (`ex_` + 64 hex); the caller room is derived from it, never stored:
//
//   room = "rm_" + hex(SHA-256(ASCII "figstudio/room/v1" || 0x00 || <32 raw bytes>))
//
// The same derivation lives in Rust (`apps/figstudio/src/mcp_pair.rs`); the fixed
// vectors in both test files must stay identical.
//
// Integration (outer Worker, before the Durable Object):
//   const proof = await checkBridgeUpgrade(request);
//   if (!proof.ok) return new Response(proof.message, { status: proof.status });
//   return env.MCP_ROOMS.get(env.MCP_ROOMS.idFromName(proof.room)).fetch(request);
//
// The executor secret is only ever offered as the second WebSocket subprotocol
// (`figstudio-exec-v1`, then the raw hex). It never appears in a URL path, query,
// error message or log; failures below name the rule, never the values.

export const ROOM_DOMAIN = "figstudio/room/v1";
export const EXEC_PROTOCOL = "figstudio-exec-v1";

const ROOM_HEX = /^[0-9a-f]{64}$/;
const BRIDGE = /^\/bridge\/(rm_[0-9a-f]{64})$/;

const fail = (status, message) => ({ ok: false, status, message });

/** `rm_<64 hex>` derived from 64 lowercase hex chars; `null` for anything else. */
export async function deriveRoom(executorHex) {
  if (typeof executorHex !== "string" || !ROOM_HEX.test(executorHex)) return null;
  const domain = new TextEncoder().encode(ROOM_DOMAIN);
  const executor = Uint8Array.from(executorHex.match(/../g), (h) => parseInt(h, 16));
  const preimage = new Uint8Array(domain.length + 1 + executor.length);
  preimage.set(domain);
  preimage[domain.length] = 0;
  preimage.set(executor, domain.length + 1);
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", preimage));
  return "rm_" + [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * Prove a `/bridge/<room>` request comes from this deployment's tab holding the
 * executor: a WebSocket upgrade, a present same-origin `Origin`, exactly two
 * subprotocols (`figstudio-exec-v1` then 64 lowercase hex) whose derived room
 * names the same Durable Object as the path. Typed result; no values are logged.
 */
export async function checkBridgeUpgrade(request) {
  const path = new URL(request.url).pathname;
  const room = BRIDGE.exec(path)?.[1];
  if (!room) return fail(404, "Not found");
  if (request.method !== "GET" ||
      request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
    return fail(426, "Use a WebSocket upgrade");
  }
  const origin = request.headers.get("Origin");
  if (!origin || origin !== new URL(request.url).origin) {
    return fail(403, "Cross-origin relay refused");
  }
  const offered = (request.headers.get("Sec-WebSocket-Protocol") ?? "")
    .split(",").map((token) => token.trim()).filter(Boolean);
  const executorHex = offered.length === 2 && offered[0] === EXEC_PROTOCOL &&
    ROOM_HEX.test(offered[1]) ? offered[1] : null;
  if (!executorHex) return fail(403, "Missing executor proof");
  if (room !== (await deriveRoom(executorHex))) return fail(403, "Wrong room for this executor");
  return { ok: true, room, executorHex };
}
