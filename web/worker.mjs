import { checkBridgeUpgrade, EXEC_PROTOCOL } from "./executor-auth.mjs";

const ROOM = /^rm_[0-9a-f]{64}$/;
const REQUEST_LIMIT = 1024 * 1024;
// The tab budgets its own serialized replies at 7.5 MiB (REPLY_BUDGET in
// apps/figstudio/src/mcp_wire.rs), so a well-formed reply stays under this cap.
const REPLY_LIMIT = 8 * 1024 * 1024;
const MAX_DOCS = 32;
const DOC_BUDGET_BYTES = 12_000;
const MAX_DOC_NAME = 96;
const MAX_LABEL = 64;
const MAX_ID = 64;
const MAX_MCP_SESSIONS = 64;
const SOCKET_CAP = 16;
const AMBIGUOUS_SESSION_CODE = -32002;
const UNSAFE_CHARS = /[\u0000-\u001f\u007f-\u009f\u200e\u200f\u202a-\u202e\u2066-\u2069]/g;
const encoder = new TextEncoder();
const owns = (body, key) => Object.hasOwn(body, key);
const object = (body) => body !== null && typeof body === "object" && !Array.isArray(body);
const validId = (id) => id === null || typeof id === "string" || typeof id === "number";

const SESSION_ARG_SCHEMA = {
  type: "string",
  description: "Target figstudio session id from `list_sessions` (required when multiple devices are connected).",
};

export const RELAY_TOOLS = [
  {
    name: "list_sessions",
    description: "List connected figstudio browser/native executor sessions in this room, their device labels, and open documents (`docs[].fileKey` is the tool `fileKey` argument; `docs[].key` is the 16-hex content SHA-256 hash).",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "select_session",
    description: "Pin a target figstudio session id (`s_…`) for subsequent tool calls from this MCP client, or pass an empty string to clear the pin.",
    inputSchema: {
      type: "object",
      properties: {
        session: {
          type: "string",
          description: "Session id from `list_sessions` (e.g. `s_a1b2c3d4e5f60718`), or `\"\"` to clear.",
        },
      },
      required: ["session"],
      additionalProperties: false,
    },
  },
];

function rpcError(status, id, code, message, data = undefined, headers = undefined) {
  const error = data === undefined ? { code, message } : { code, message, data };
  const init = headers ? { status, headers } : { status };
  return Response.json({ jsonrpc: "2.0", id, error }, init);
}

function cleanStr(val, maxLen) {
  if (typeof val !== "string") return "";
  const cleaned = val.replace(/[\r\n\t]+/g, " ").replace(UNSAFE_CHARS, "").trim();
  return Array.from(cleaned).slice(0, maxLen).join("");
}

function sanitizeDocs(rawDocs) {
  if (!Array.isArray(rawDocs)) return [];
  const candidates = [];
  for (const item of rawDocs.slice(0, MAX_DOCS)) {
    if (!object(item)) continue;
    const base = {
      tab: Number.isInteger(item.tab) && item.tab >= 0 ? item.tab : candidates.length,
      name: cleanStr(item.name, MAX_DOC_NAME) || "untitled",
    };
    const key = cleanStr(item.key, 32);
    if (key) base.key = key;
    if (Number.isInteger(item.pages) && item.pages >= 0) base.pages = item.pages;
    const baseBytes = encoder.encode(JSON.stringify(base)).byteLength + 1;
    const hasFileKey = typeof item.fileKey === "string" && item.fileKey.length > 0;
    const full = hasFileKey ? { ...base, fileKey: item.fileKey } : base;
    const fullBytes = hasFileKey
      ? encoder.encode(JSON.stringify(full)).byteLength + 1
      : baseBytes;
    candidates.push({ base, baseBytes, full, fullBytes, hasFileKey });
  }
  const out = [];
  let usedBytes = 2;
  for (let i = 0; i < candidates.length; i++) {
    const cur = candidates[i];
    if (usedBytes + cur.fullBytes <= DOC_BUDGET_BYTES) {
      out.push(cur.full);
      usedBytes += cur.fullBytes;
      continue;
    }
    if (usedBytes + cur.baseBytes > DOC_BUDGET_BYTES) continue;
    // Store the row without `fileKey` if that keyless row still leaves room for the
    // later rows that fit; otherwise skip, so a keyless row never drops a tail document.
    let laterBytes = 0;
    for (let j = i + 1; j < candidates.length; j++) {
      const next = candidates[j];
      if (usedBytes + laterBytes + next.fullBytes <= DOC_BUDGET_BYTES) {
        laterBytes += next.fullBytes;
      } else if (usedBytes + laterBytes + next.baseBytes <= DOC_BUDGET_BYTES) {
        laterBytes += next.baseBytes;
      }
    }
    if (usedBytes + cur.baseBytes + laterBytes <= DOC_BUDGET_BYTES) {
      out.push(cur.base);
      usedBytes += cur.baseBytes;
    }
  }
  return out;
}

const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

function rotr(x, n) {
  return (x >>> n) | (x << (32 - n));
}

// Synchronous SHA-256 hex digest so `webSocketMessage` stays synchronous while deriving
// collision-resistant session identifiers.
export function sha256Hex(text) {
  const bytes = encoder.encode(text);
  const bitLen = bytes.length * 8;
  const totalLen = (((bytes.length + 8) >> 6) + 1) << 6;
  const padded = new Uint8Array(totalLen);
  padded.set(bytes);
  padded[bytes.length] = 0x80;
  const view = new DataView(padded.buffer);
  view.setUint32(totalLen - 8, Math.floor(bitLen / 0x100000000), false);
  view.setUint32(totalLen - 4, bitLen >>> 0, false);

  let h0 = 0x6a09e667, h1 = 0xbb67ae85, h2 = 0x3c6ef372, h3 = 0xa54ff53a;
  let h4 = 0x510e527f, h5 = 0x9b05688c, h6 = 0x1f83d9ab, h7 = 0x5be0cd19;
  const w = new Uint32Array(64);

  for (let offset = 0; offset < totalLen; offset += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(offset + i * 4, false);
    for (let i = 16; i < 64; i++) {
      const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3);
      const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
      w[i] = (w[i - 16] + s0 + w[i - 7] + s1) >>> 0;
    }
    let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7;
    for (let i = 0; i < 64; i++) {
      const S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
      const ch = (e & f) ^ (~e & g);
      const temp1 = (h + S1 + ch + SHA256_K[i] + w[i]) >>> 0;
      const S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
      const maj = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (S0 + maj) >>> 0;
      h = g; g = f; f = e; e = (d + temp1) >>> 0;
      d = c; c = b; b = a; a = (temp1 + temp2) >>> 0;
    }
    h0 = (h0 + a) >>> 0; h1 = (h1 + b) >>> 0; h2 = (h2 + c) >>> 0; h3 = (h3 + d) >>> 0;
    h4 = (h4 + e) >>> 0; h5 = (h5 + f) >>> 0; h6 = (h6 + g) >>> 0; h7 = (h7 + h) >>> 0;
  }
  return [h0, h1, h2, h3, h4, h5, h6, h7]
    .map((n) => n.toString(16).padStart(8, "0"))
    .join("");
}

// 128-bit SHA-256 session identifier (`s_<32 hex>`) derived from `(device.id, instance)`.
export function deriveSessionId(deviceId, instance) {
  return `s_${sha256Hex(`figstudio/session/v1\0${deviceId}\0${instance}`).slice(0, 32)}`;
}

function parseReadyV2(reply, order) {
  const dev = object(reply.device) ? reply.device : {};
  const deviceId = cleanStr(dev.id, MAX_ID) || `anon_${order}`;
  const deviceLabel = cleanStr(dev.label, MAX_LABEL) || "Browser tab";
  const instance = cleanStr(reply.instance, MAX_ID) || String(order);
  const focused = reply.focused === true;
  const docs = sanitizeDocs(reply.docs);
  const sessionId = deriveSessionId(deviceId, instance);
  return {
    sessionId,
    deviceId,
    deviceLabel,
    instance,
    focused,
    docs,
    readyAt: Date.now(),
  };
}

// The page asks `GET /mcp/` whether its origin runs this relay (CloudRelay in
// apps/figstudio/src/mcp_wire.rs). Only this Worker sets the header; a static-only
// deploy answers the same path with a missing asset or a fallback page, so the app hides
// Cloud relay there instead of opening a socket that cannot connect. Not exported: the
// Workers runtime takes every named export of the main module as an entrypoint and
// refuses to start on a string.
const RELAY_PROBE_HEADER = "figstudio-relay";

function route(request) {
  const path = new URL(request.url).pathname;
  if (path === "/mcp/" && (request.method === "GET" || request.method === "HEAD")) {
    return { probe: true };
  }
  const match = /^\/(mcp|bridge)\/([^/]+)$/.exec(path);
  if (match && ROOM.test(match[2])) return { kind: match[1], room: match[2] };
  if (/^\/(mcp|bridge)(\/|$)/.test(path)) return { invalid: true };
  return null;
}

export default {
  async fetch(request, env) {
    const target = route(request);
    if (!target) return env.ASSETS.fetch(request);
    if (target.probe) {
      return new Response(null, {
        status: 204,
        headers: { [RELAY_PROBE_HEADER]: "1", "Cache-Control": "no-store" },
      });
    }
    if (target.invalid) return new Response("Not found", { status: 404 });
    // The bridge is the executor capability: prove the executor secret before the room
    // names a Durable Object, so a caller that only knows the room (the MCP URL) cannot
    // create or reach one. `/mcp/<room>` stays caller-only — that side's secret is the
    // room id itself.
    if (target.kind === "bridge") {
      const proof = await checkBridgeUpgrade(request);
      if (!proof.ok) return new Response(proof.message, { status: proof.status });
      return env.MCP_ROOMS.get(env.MCP_ROOMS.idFromName(proof.room)).fetch(request);
    }
    return env.MCP_ROOMS.get(env.MCP_ROOMS.idFromName(target.room)).fetch(request);
  },
};

async function readBody(request) {
  if (Number(request.headers.get("content-length")) > REQUEST_LIMIT) {
    throw new RangeError("Request too large");
  }
  if (!request.body) return "";
  const reader = request.body.getReader();
  const decoder = new TextDecoder();
  let bytes = 0;
  let text = "";
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return text + decoder.decode();
      bytes += value.byteLength;
      if (bytes > REQUEST_LIMIT) {
        await reader.cancel();
        throw new RangeError("Request too large");
      }
      text += decoder.decode(value, { stream: true });
    }
  } finally {
    reader.releaseLock();
  }
}

export class McpRoom {
  constructor(ctx) {
    this.ctx = ctx;
    this.pending = new Map();
    // Server-issued `Mcp-Session-Id` -> pinned figstudio session (`string | null`),
    // capped at MAX_MCP_SESSIONS (LRU). Client-supplied ids that were not minted on
    // `initialize` are ignored.
    this.mcpSessions = new Map();
    this.nativeSessionTools = new Set();
    // No storage: a hard eviction tears down the isolate with its timers and pending
    // map, so those callers' connections drop (they see no 504 from this deadline); the
    // 30 s deadline bounds only the requests that survive in this isolate.
    const restored = ctx.getWebSockets("tab");
    // A socket is authenticated only by the marker its `connect` wrote after proving the
    // executor secret. Attachments restored from an earlier deploy (`{order}`, `{ready,
    // order}`) carry no marker — the old deploy proved nothing this deploy accepts — so
    // they are closed here: a re-sent `ready` (which the old client sends on tab or
    // selection changes) must never restore the executor role. The marker is a constant
    // protocol name, never the secret.
    for (const ws of restored) {
      if (ws.deserializeAttachment()?.authenticatedProtocol !== EXEC_PROTOCOL) {
        try { ws.close(1008, "Reconnect to re-authenticate"); } catch { /* already closing */ }
      }
    }
    this.order = Math.max(0, ...restored.map((ws) => ws.deserializeAttachment()?.order ?? 0));
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  mintMcpSession() {
    const id = crypto.randomUUID();
    if (this.mcpSessions.size >= MAX_MCP_SESSIONS) {
      const oldest = this.mcpSessions.keys().next().value;
      if (oldest !== undefined) this.mcpSessions.delete(oldest);
    }
    this.mcpSessions.set(id, null);
    return id;
  }

  resolveMcpSession(request) {
    const raw = request.headers.get("Mcp-Session-Id")?.trim() ?? "";
    if (!raw) return { callerKey: null, unrecognized: false };
    if (!this.mcpSessions.has(raw)) return { callerKey: null, unrecognized: true };
    const pinned = this.mcpSessions.get(raw);
    this.mcpSessions.delete(raw);
    this.mcpSessions.set(raw, pinned);
    return { callerKey: raw, unrecognized: false };
  }

  readyTabs() {
    // Only sockets this deploy authenticated (the marker) that also said `ready`: a
    // connected-but-silent client (or any socket restored without the marker, such as
    // one stored by an earlier deploy) never takes the executor role, so an
    // admitted-but-unproved tab cannot receive the next RPC.
    return this.ctx.getWebSockets("tab")
      .filter((ws) => ws.readyState === 1 &&
        ws.deserializeAttachment()?.authenticatedProtocol === EXEC_PROTOCOL &&
        ws.deserializeAttachment()?.ready === true)
      .sort((a, b) => (b.deserializeAttachment()?.order ?? 0) -
        (a.deserializeAttachment()?.order ?? 0));
  }

  activeTab() {
    return this.readyTabs()[0];
  }

  deviceKey(ws, index = 0) {
    const att = ws.deserializeAttachment() ?? {};
    if (typeof att.deviceId === "string" && att.deviceId.length > 0) {
      return `v2:${att.deviceId}`;
    }
    // Each protocol-1 socket gets its own device key so two legacy machines trigger rule 4.
    const baseOrder = att.firstOrder ?? att.order ?? 0;
    return `v1:${baseOrder}:${index}`;
  }

  sessionSummary(ws, now = Date.now()) {
    const att = ws.deserializeAttachment() ?? {};
    const baseOrder = att.firstOrder ?? att.order ?? 0;
    const session = att.sessionId ?? deriveSessionId("v1", String(baseOrder));
    const deviceLabel = att.deviceLabel ?? "Browser tab";
    const focused = att.focused === true;
    const docs = Array.isArray(att.docs) ? att.docs : [];
    const secondsSinceReady = typeof att.readyAt === "number"
      ? Math.max(0, Math.round((now - att.readyAt) / 1000))
      : null;
    // Never publish `device.id` in `list_sessions` or `error.data`: until per-device
    // credentials exist, a published `device.id` would let another executor in the room
    // copy it and join the same device group.
    return {
      session,
      device: { label: deviceLabel },
      focused,
      docs,
      secondsSinceReady,
    };
  }

  listSessionsPayload(callerKey) {
    const now = Date.now();
    const ready = this.readyTabs();
    const openCount = this.ctx.getWebSockets("tab").filter((ws) => ws.readyState === 1).length;
    const sticky = callerKey ? (this.mcpSessions.get(callerKey) ?? null) : null;
    return {
      sessions: ready.map((ws) => this.sessionSummary(ws, now)),
      selectedSession: sticky,
      socketsInUse: openCount,
      socketCap: SOCKET_CAP,
    };
  }

  resolveToolTarget(body, callerKey) {
    const ready = this.readyTabs();
    const headers = callerKey ? { "Mcp-Session-Id": callerKey } : undefined;
    if (ready.length === 0) {
      return { error: rpcError(503, body.id, -32000, "No browser tab connected", undefined, headers) };
    }
    const toolName = body.params?.name;
    const isNativeSessionTool = typeof toolName === "string" && this.nativeSessionTools.has(toolName);
    const args = object(body.params?.arguments) ? body.params.arguments : null;
    const explicitSession = !isNativeSessionTool && args && typeof args.session === "string"
      ? args.session.trim()
      : "";

    // Rule 1: explicit `arguments.session`
    if (explicitSession) {
      const matches = ready.filter((ws) => this.sessionSummary(ws).session === explicitSession);
      if (matches.length === 0) {
        const data = this.listSessionsPayload(callerKey);
        return {
          error: rpcError(
            200,
            body.id,
            AMBIGUOUS_SESSION_CODE,
            `Session '${explicitSession}' is not connected; choose one of the live sessions`,
            data,
            headers,
          ),
        };
      }
      if (matches.length > 1) {
        const data = this.listSessionsPayload(callerKey);
        return {
          error: rpcError(
            200,
            body.id,
            AMBIGUOUS_SESSION_CODE,
            `Session '${explicitSession}' matches multiple executors; cannot route unambiguously`,
            data,
            headers,
          ),
        };
      }
      const nextArgs = { ...args };
      delete nextArgs.session;
      const forwardedBody = {
        ...body,
        params: { ...body.params, arguments: nextArgs },
      };
      return { tab: matches[0], body: forwardedBody };
    }

    // Rule 2: sticky choice for this caller (`Mcp-Session-Id` issued on `initialize`)
    const stickySession = callerKey ? this.mcpSessions.get(callerKey) : null;
    if (stickySession) {
      const matches = ready.filter((ws) => this.sessionSummary(ws).session === stickySession);
      if (matches.length === 1) return { tab: matches[0], body };
      if (matches.length > 1) {
        const data = this.listSessionsPayload(callerKey);
        return {
          error: rpcError(
            200,
            body.id,
            AMBIGUOUS_SESSION_CODE,
            `Pinned session '${stickySession}' matches multiple executors; cannot route unambiguously`,
            data,
            headers,
          ),
        };
      }
      // Do not fall through to another device when a pinned session disconnects.
      const data = this.listSessionsPayload(callerKey);
      return {
        error: rpcError(
          200,
          body.id,
          AMBIGUOUS_SESSION_CODE,
          `Pinned session '${stickySession}' is no longer connected; call \`select_session\` or pass \`session\``,
          data,
          headers,
        ),
      };
    }

    // Rule 3: if all ready sessions belong to a single device, pick its newest focused tab
    // (or newest ready tab), unless two live sockets collide on the same session id.
    // `deviceLabel` is display-only and is not a routing credential: tabs on the same
    // `deviceId` route to the newest focused tab even when their labels differ.
    const devices = new Set(ready.map((ws, idx) => this.deviceKey(ws, idx)));
    if (devices.size <= 1) {
      const sessionIds = new Set(ready.map((ws) => this.sessionSummary(ws).session));
      if (sessionIds.size === ready.length) {
        const focusedTab = ready.find((ws) => ws.deserializeAttachment()?.focused === true);
        return { tab: focusedTab ?? ready[0], body };
      }
    }

    // Rule 4: sessions on multiple devices (or conflicting claims) without a choice -> -32002
    const data = this.listSessionsPayload(callerKey);
    return {
      error: rpcError(
        200,
        body.id,
        AMBIGUOUS_SESSION_CODE,
        "Several figstudio sessions are connected across devices; pass `session` or call `select_session`",
        data,
        headers,
      ),
    };
  }

  handleRelayTool(body, callerKey) {
    const name = body.params?.name;
    const headers = callerKey ? { "Mcp-Session-Id": callerKey } : undefined;
    const init = headers ? { headers } : undefined;
    if (name === "list_sessions") {
      const payload = this.listSessionsPayload(callerKey);
      return Response.json({
        jsonrpc: "2.0",
        id: body.id,
        result: {
          content: [{ type: "text", text: JSON.stringify(payload, null, 2) }],
          structuredContent: payload,
        },
      }, init);
    }
    if (name === "select_session") {
      const args = object(body.params?.arguments) ? body.params.arguments : {};
      if (typeof args.session !== "string") {
        return rpcError(400, body.id, -32602, "`select_session` requires a string `session` argument", undefined, headers);
      }
      if (!callerKey) {
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            isError: true,
            content: [{
              type: "text",
              text: "No server-issued `Mcp-Session-Id` header on this request; call `initialize` first or pass `session` on each tool call.",
            }],
          },
        });
      }
      const target = args.session.trim();
      if (!target) {
        this.mcpSessions.set(callerKey, null);
        const payload = this.listSessionsPayload(callerKey);
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            content: [{ type: "text", text: JSON.stringify({ cleared: true, ...payload }, null, 2) }],
          },
        }, init);
      }
      const matches = this.readyTabs().filter((ws) => this.sessionSummary(ws).session === target);
      if (matches.length === 0) {
        const payload = this.listSessionsPayload(callerKey);
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            isError: true,
            content: [{
              type: "text",
              text: `Session '${target}' is not connected. Live sessions: ${JSON.stringify(payload.sessions)}`,
            }],
          },
        }, init);
      }
      if (matches.length > 1) {
        const payload = this.listSessionsPayload(callerKey);
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            isError: true,
            content: [{
              type: "text",
              text: `Session '${target}' matches multiple executors; cannot pin an ambiguous session. Live sessions: ${JSON.stringify(payload.sessions)}`,
            }],
          },
        }, init);
      }
      this.mcpSessions.set(callerKey, target);
      const chosen = this.sessionSummary(matches[0]);
      return Response.json({
        jsonrpc: "2.0",
        id: body.id,
        result: {
          content: [{ type: "text", text: JSON.stringify({ selected: chosen }, null, 2) }],
        },
      }, init);
    }
    return null;
  }

  async fetch(request) {
    const target = route(request);
    if (!target || target.invalid) return new Response("Not found", { status: 404 });
    const origin = request.headers.get("Origin");
    if (origin && origin !== new URL(request.url).origin) {
      return new Response("Cross-origin relay refused", { status: 403 });
    }
    if (target.kind === "bridge") return this.connect(request);
    if (request.method !== "POST") {
      return new Response("Use POST; SSE is not supported", {
        status: 405, headers: { Allow: "POST" },
      });
    }
    let body;
    try {
      body = JSON.parse(await readBody(request));
    } catch (error) {
      return error instanceof RangeError
        ? rpcError(413, null, -32600, "Request exceeds 1 MiB")
        : rpcError(400, null, -32700, "Invalid JSON");
    }
    if (Array.isArray(body)) return rpcError(400, null, -32600, "Batch requests are not supported");
    if (!object(body) || body.jsonrpc !== "2.0" ||
        (owns(body, "id") && !validId(body.id))) {
      return rpcError(400, null, -32600, "Invalid JSON-RPC message");
    }
    const { callerKey, unrecognized } = this.resolveMcpSession(request);
    // Streamable HTTP (2025-03-26): an unknown or expired `Mcp-Session-Id` header on any
    // non-`initialize` message (including notifications) must be answered with HTTP 404 so
    // the client starts a new session, and must never return 202 or fall through to Rule 3.
    if (unrecognized && body.method !== "initialize") {
      return rpcError(
        404,
        owns(body, "id") ? body.id : null,
        AMBIGUOUS_SESSION_CODE,
        "Unrecognized or expired `Mcp-Session-Id`; start a new session with `initialize`",
        this.listSessionsPayload(null),
      );
    }
    if (!owns(body, "method") && owns(body, "id") &&
        (owns(body, "result") !== owns(body, "error"))) {
      return new Response(null, { status: 202 });
    }
    if (typeof body.method !== "string" || owns(body, "result") || owns(body, "error")) {
      return rpcError(400, body.id ?? null, -32600, "Invalid JSON-RPC request");
    }
    if (!owns(body, "id")) return new Response(null, { status: 202 });

    const headers = callerKey ? { "Mcp-Session-Id": callerKey } : undefined;

    if (body.method === "tools/call") {
      const localReply = this.handleRelayTool(body, callerKey);
      if (localReply) return localReply;
    }

    let tab;
    let forwardBody = body;
    if (body.method === "tools/call") {
      const resolved = this.resolveToolTarget(body, callerKey);
      if (resolved.error) return resolved.error;
      tab = resolved.tab;
      forwardBody = resolved.body;
    } else {
      tab = this.activeTab();
      if (!tab) {
        if (body.method === "tools/list") {
          return Response.json(
            { jsonrpc: "2.0", id: body.id, result: { tools: RELAY_TOOLS } },
            headers ? { headers } : undefined,
          );
        }
        return rpcError(503, body.id, -32000, "No browser tab connected", undefined, headers);
      }
    }

    if (this.pending.size >= 128) {
      return rpcError(503, body.id, -32000, "Relay is busy; retry later", undefined, headers);
    }
    const id = crypto.randomUUID();
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.finish(id, rpcError(504, body.id, -32000, "Browser tab timed out", undefined, headers));
      }, 30_000);
      this.pending.set(id, { tab, rpcId: body.id, method: body.method, callerKey, timer, resolve });
      try {
        tab.send(JSON.stringify({ type: "rpc", id, body: forwardBody }));
      } catch {
        this.failTab(tab);
      }
    });
  }

  async connect(request) {
    // Defense in depth: the outer Worker proved this upgrade, but the Durable Object
    // re-verifies the same proof itself (one SHA-256) rather than trusting any header
    // that merely claims the check already happened — such a header is forgeable.
    const proof = await checkBridgeUpgrade(request);
    if (!proof.ok) return new Response(proof.message, { status: proof.status });
    if (this.ctx.getWebSockets("tab").filter((ws) => ws.readyState === 1).length >= SOCKET_CAP) {
      return new Response("Too many browser tabs connected", { status: 503 });
    }
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, ["tab"]);
    // The marker is the non-secret proof record: written only here, after the
    // re-verified upgrade, so a socket that never went through this `connect` (an
    // earlier deploy's restore) can never claim it.
    server.serializeAttachment({ authenticatedProtocol: EXEC_PROTOCOL, ready: false, order: ++this.order });
    // Echo only the constant marker — never the offered executor hex — so clients that
    // expect the selected subprotocol to be answered see the handshake complete.
    return new Response(null, {
      status: 101,
      webSocket: client,
      headers: { "Sec-WebSocket-Protocol": EXEC_PROTOCOL },
    });
  }

  finish(id, response) {
    const entry = this.pending.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    this.pending.delete(id);
    entry.resolve(response);
  }

  failTab(tab) {
    for (const [id, entry] of this.pending) {
      if (entry.tab === tab) {
        this.finish(id, rpcError(503, entry.rpcId, -32000, "Browser tab disconnected"));
      }
    }
  }

  decorateReply(tab, entry, body) {
    if (!object(body.result)) return body;
    if (entry.method === "tools/list" && Array.isArray(body.result.tools)) {
      const existingNames = new Set(body.result.tools.map((t) => t?.name));
      const augmented = body.result.tools.map((tool) => {
        if (!object(tool) || !object(tool.inputSchema)) return tool;
        const props = object(tool.inputSchema.properties) ? tool.inputSchema.properties : {};
        if (owns(props, "session")) {
          if (typeof tool.name === "string") this.nativeSessionTools.add(tool.name);
          return tool;
        }
        if (typeof tool.name === "string") this.nativeSessionTools.delete(tool.name);
        return {
          ...tool,
          inputSchema: {
            ...tool.inputSchema,
            properties: { ...props, session: SESSION_ARG_SCHEMA },
          },
        };
      });
      for (const relayTool of RELAY_TOOLS) {
        if (!existingNames.has(relayTool.name)) augmented.push(relayTool);
      }
      return { ...body, result: { ...body.result, tools: augmented } };
    }
    if (entry.method === "tools/call") {
      const summary = this.sessionSummary(tab);
      const prevMeta = object(body.result._meta) ? body.result._meta : {};
      return {
        ...body,
        result: {
          ...body.result,
          _meta: {
            ...prevMeta,
            "figstudio/session": summary.session,
            "figstudio/device": summary.device.label,
          },
        },
      };
    }
    return body;
  }

  webSocketMessage(tab, message) {
    // Every frame requires the proof marker before any processing — `ready` alone (with
    // or without a client-claimed `auth` field) never grants or restores it. This also
    // covers any unmarked attachment that escaped the constructor's close (defense in
    // depth): the socket is closed and its pending calls failed, not promoted.
    const prev = tab.deserializeAttachment();
    if (prev?.authenticatedProtocol !== EXEC_PROTOCOL) {
      this.failTab(tab);
      try { tab.close(1008, "Reconnect to re-authenticate"); } catch { /* already closing */ }
      return;
    }
    if (typeof message !== "string") {
      this.failTab(tab);
      tab.close(1009, "Invalid or oversized relay frame");
      return;
    }
    // A UTF-16 code unit encodes to at most 3 UTF-8 bytes, so short frames skip the encode.
    if (message.length * 3 > REPLY_LIMIT && encoder.encode(message).byteLength > REPLY_LIMIT) {
      this.oversized(tab, message);
      return;
    }
    let reply;
    try { reply = JSON.parse(message); } catch { return; }
    if (!object(reply)) return;
    if (reply.type === "ready" && reply.protocol === 1) {
      const order = ++this.order;
      if (prev.ready === true) {
        const candidate = prev.firstOrder ?? prev.order ?? order;
        const collides = this.ctx.getWebSockets("tab").some((other) => {
          if (other === tab) return false;
          const otherAtt = other.deserializeAttachment();
          return (otherAtt?.firstOrder ?? otherAtt?.order) === candidate;
        });
        const firstOrder = collides ? order : candidate;
        tab.serializeAttachment({
          authenticatedProtocol: EXEC_PROTOCOL,
          ready: true,
          order,
          firstOrder,
          sessionId: prev.sessionId ?? deriveSessionId("v1", String(firstOrder)),
        });
      } else {
        // First `ready` on a newly connected socket keeps the minimal attachment shape.
        tab.serializeAttachment({ authenticatedProtocol: EXEC_PROTOCOL, ready: true, order });
      }
      return;
    }
    if (reply.type === "ready" && reply.protocol === 2) {
      const order = ++this.order;
      const v2 = parseReadyV2(reply, order);
      tab.serializeAttachment({
        authenticatedProtocol: EXEC_PROTOCOL,
        ready: true,
        order,
        ...v2,
      });
      return;
    }
    if (reply.type !== "rpc_reply") return;
    const entry = this.pending.get(reply.id);
    if (!entry || entry.tab !== tab) return;
    // The reply contract lives in rpc_reply (apps/figstudio/src/mcp_wire.rs): status is
    // always present -- 200 (or any status) with a JSON-RPC object body whose id matches
    // the caller's, or 202 with a null body for a notification. A null-body 202 is the
    // only frame without the caller's id, so it is accepted on status alone.
    const status = reply.status;
    if (!Number.isInteger(status) || status < 200 || status > 599 || status === 204 ||
        status === 205 || status === 304 ||
        (status === 202 ? reply.body !== null
          : !object(reply.body) || reply.body.jsonrpc !== "2.0" ||
            reply.body.id !== entry.rpcId ||
            (owns(reply.body, "result") === owns(reply.body, "error")))) {
      this.finish(reply.id, rpcError(502, entry.rpcId, -32000, "Invalid browser reply"));
      return;
    }
    // A 202/null frame is the notification ack: the caller gets the same body-less 202
    // the direct path returns, not a literal JSON `null` body.
    if (status === 202) {
      this.finish(reply.id, new Response(null, { status: 202 }));
      return;
    }
    const decorated = this.decorateReply(tab, entry, reply.body);
    let sessionHeader = entry.callerKey;
    if (entry.method === "initialize" && status === 200 && owns(decorated, "result")) {
      sessionHeader = this.mintMcpSession();
    }
    const init = sessionHeader
      ? { status, headers: { "Mcp-Session-Id": sessionHeader } }
      : { status };
    this.finish(reply.id, Response.json(decorated, init));
  }

  // One oversized rpc_reply fails only its matching call (the tab should have replaced it;
  // see REPLY_BUDGET in apps/figstudio/src/mcp_wire.rs); any other oversized frame still
  // fails every pending call on the socket.
  oversized(tab, message) {
    try {
      const reply = JSON.parse(message);
      const entry = object(reply) && reply.type === "rpc_reply"
        ? this.pending.get(reply.id) : undefined;
      if (entry?.tab === tab) {
        this.finish(reply.id, rpcError(502, entry.rpcId, -32000, "Browser reply exceeded 8 MiB"));
        return;
      }
    } catch { /* not JSON: fall through to closing the socket */ }
    this.failTab(tab);
    tab.close(1009, "Invalid or oversized relay frame");
  }

  webSocketClose(tab) {
    this.failTab(tab);
    tab.close(1000, "Bridge closed");
  }

  webSocketError(tab) {
    this.failTab(tab);
    tab.close(1011, "Bridge error");
  }
}
