import { checkBridgeUpgrade, EXEC_PROTOCOL } from "./executor-auth.mjs";

const ROOM = /^rm_[0-9a-f]{64}$/;
const REQUEST_LIMIT = 1024 * 1024;
// The tab budgets its own serialized replies at 7.5 MiB (REPLY_BUDGET in
// apps/figstudio/src/mcp_wire.rs), so a well-formed reply stays under this cap.
const REPLY_LIMIT = 8 * 1024 * 1024;
const MAX_DOCS = 32;
const MAX_DOC_NAME = 96;
const MAX_LABEL = 64;
const MAX_ID = 64;
const AMBIGUOUS_SESSION_CODE = -32002;
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
    description: "List connected figstudio browser/native executor sessions in this room, their device labels, and open documents.",
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
          description: "Session id from `list_sessions` (e.g. `s_a1b2c3d4`), or `\"\"` to clear.",
        },
      },
      required: ["session"],
      additionalProperties: false,
    },
  },
];

function rpcError(status, id, code, message, data = undefined) {
  const error = data === undefined ? { code, message } : { code, message, data };
  return Response.json({ jsonrpc: "2.0", id, error }, { status });
}

function cleanStr(val, maxLen) {
  if (typeof val !== "string") return "";
  return val.trim().slice(0, maxLen);
}

function sanitizeDocs(rawDocs) {
  if (!Array.isArray(rawDocs)) return [];
  const out = [];
  for (const item of rawDocs.slice(0, MAX_DOCS)) {
    if (!object(item)) continue;
    const entry = {
      tab: Number.isInteger(item.tab) && item.tab >= 0 ? item.tab : out.length,
      name: cleanStr(item.name, MAX_DOC_NAME) || "untitled",
    };
    const key = cleanStr(item.key, 32);
    if (key) entry.key = key;
    if (Number.isInteger(item.pages) && item.pages >= 0) entry.pages = item.pages;
    out.push(entry);
  }
  return out;
}

// Fast, deterministic 8-hex session hash (FNV-1a 32-bit + mix) so `webSocketMessage` stays
// synchronous while producing a stable `s_<8 hex>` identifier from `(device.id, instance)`.
export function deriveSessionId(deviceId, instance) {
  const input = `${deviceId}\0${instance}`;
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < input.length; i++) {
    const c = input.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 0x01000193) >>> 0;
    h2 = Math.imul(h2 ^ (c + i), 0x85ebca6b) >>> 0;
  }
  const mixed = ((h1 ^ (h2 >>> 13)) >>> 0).toString(16).padStart(8, "0");
  return `s_${mixed}`;
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

function route(request) {
  const path = new URL(request.url).pathname;
  const match = /^\/(mcp|bridge)\/([^/]+)$/.exec(path);
  if (match && ROOM.test(match[2])) return { kind: match[1], room: match[2] };
  if (/^\/(mcp|bridge)(\/|$)/.test(path)) return { invalid: true };
  return null;
}

export default {
  async fetch(request, env) {
    const target = route(request);
    if (!target) return env.ASSETS.fetch(request);
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
    this.sticky = new Map();
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

  sessionSummary(ws, now = Date.now()) {
    const att = ws.deserializeAttachment() ?? {};
    const order = att.order ?? 0;
    const session = att.sessionId ?? `s_v1_${order}`;
    const deviceId = att.deviceId ?? "";
    const deviceLabel = att.deviceLabel ?? "Browser tab";
    const focused = att.focused === true;
    const docs = Array.isArray(att.docs) ? att.docs : [];
    const secondsSinceReady = typeof att.readyAt === "number"
      ? Math.max(0, Math.round((now - att.readyAt) / 1000))
      : null;
    return {
      session,
      device: { id: deviceId || "legacy", label: deviceLabel },
      focused,
      docs,
      secondsSinceReady,
    };
  }

  listSessionsPayload(callerKey) {
    const now = Date.now();
    const ready = this.readyTabs();
    const openCount = this.ctx.getWebSockets("tab").filter((ws) => ws.readyState === 1).length;
    const sticky = this.sticky.get(callerKey) ?? null;
    return {
      sessions: ready.map((ws) => this.sessionSummary(ws, now)),
      selectedSession: sticky,
      socketsInUse: openCount,
      socketCap: 16,
    };
  }

  resolveToolTarget(body, callerKey) {
    const ready = this.readyTabs();
    if (ready.length === 0) {
      return { error: rpcError(503, body.id, -32000, "No browser tab connected") };
    }
    const args = object(body.params?.arguments) ? body.params.arguments : null;
    const explicitSession = args && typeof args.session === "string" ? args.session.trim() : "";

    // Rule 1: explicit `arguments.session`
    if (explicitSession) {
      const match = ready.find((ws) => this.sessionSummary(ws).session === explicitSession);
      if (!match) {
        const data = this.listSessionsPayload(callerKey);
        return {
          error: rpcError(
            200,
            body.id,
            AMBIGUOUS_SESSION_CODE,
            `Session '${explicitSession}' is not connected; choose one of the live sessions`,
            data,
          ),
        };
      }
      const nextArgs = { ...args };
      delete nextArgs.session;
      const forwardedBody = {
        ...body,
        params: { ...body.params, arguments: nextArgs },
      };
      return { tab: match, body: forwardedBody };
    }

    // Rule 2: sticky choice for this caller
    const stickySession = this.sticky.get(callerKey);
    if (stickySession) {
      const match = ready.find((ws) => this.sessionSummary(ws).session === stickySession);
      if (match) return { tab: match, body };
      this.sticky.delete(callerKey);
    }

    // Rule 3: if all ready sessions belong to a single device, pick its focused or newest tab
    const devices = new Set(ready.map((ws) => ws.deserializeAttachment()?.deviceId ?? ""));
    if (devices.size <= 1) {
      const focusedTab = ready.find((ws) => ws.deserializeAttachment()?.focused === true);
      return { tab: focusedTab ?? ready[0], body };
    }

    // Rule 4: sessions on multiple devices without a choice -> -32002 with candidate list
    const data = this.listSessionsPayload(callerKey);
    return {
      error: rpcError(
        200,
        body.id,
        AMBIGUOUS_SESSION_CODE,
        "Several figstudio sessions are connected across devices; pass `session` or call `select_session`",
        data,
      ),
    };
  }

  handleRelayTool(body, callerKey) {
    const name = body.params?.name;
    if (name === "list_sessions") {
      const payload = this.listSessionsPayload(callerKey);
      return Response.json({
        jsonrpc: "2.0",
        id: body.id,
        result: {
          content: [{ type: "text", text: JSON.stringify(payload, null, 2) }],
          structuredContent: payload,
        },
      });
    }
    if (name === "select_session") {
      const args = object(body.params?.arguments) ? body.params.arguments : {};
      if (typeof args.session !== "string") {
        return rpcError(400, body.id, -32602, "`select_session` requires a string `session` argument");
      }
      const target = args.session.trim();
      if (!target) {
        this.sticky.delete(callerKey);
        const payload = this.listSessionsPayload(callerKey);
        return Response.json({
          jsonrpc: "2.0",
          id: body.id,
          result: {
            content: [{ type: "text", text: JSON.stringify({ cleared: true, ...payload }, null, 2) }],
          },
        });
      }
      const match = this.readyTabs().find((ws) => this.sessionSummary(ws).session === target);
      if (!match) {
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
        });
      }
      this.sticky.set(callerKey, target);
      const chosen = this.sessionSummary(match);
      return Response.json({
        jsonrpc: "2.0",
        id: body.id,
        result: {
          content: [{ type: "text", text: JSON.stringify({ selected: chosen }, null, 2) }],
        },
      });
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
    if (!owns(body, "method") && owns(body, "id") &&
        (owns(body, "result") !== owns(body, "error"))) {
      return new Response(null, { status: 202 });
    }
    if (typeof body.method !== "string" || owns(body, "result") || owns(body, "error")) {
      return rpcError(400, body.id ?? null, -32600, "Invalid JSON-RPC request");
    }
    if (!owns(body, "id")) return new Response(null, { status: 202 });

    const callerKey = request.headers.get("Mcp-Session-Id")?.trim() || "default";

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
          return Response.json({ jsonrpc: "2.0", id: body.id, result: { tools: RELAY_TOOLS } });
        }
        return rpcError(503, body.id, -32000, "No browser tab connected");
      }
    }

    if (this.pending.size >= 128) {
      return rpcError(503, body.id, -32000, "Relay is busy; retry later");
    }
    const id = crypto.randomUUID();
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.finish(id, rpcError(504, body.id, -32000, "Browser tab timed out"));
      }, 30_000);
      this.pending.set(id, { tab, rpcId: body.id, method: body.method, timer, resolve });
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
    if (this.ctx.getWebSockets("tab").filter((ws) => ws.readyState === 1).length >= 16) {
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
        if (owns(props, "session")) return tool;
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
      const att = tab.deserializeAttachment() ?? {};
      if (att.sessionId) {
        const prevMeta = object(body.result._meta) ? body.result._meta : {};
        return {
          ...body,
          result: {
            ...body.result,
            _meta: {
              ...prevMeta,
              "figstudio/session": att.sessionId,
              "figstudio/device": att.deviceLabel ?? "Browser tab",
            },
          },
        };
      }
    }
    return body;
  }

  webSocketMessage(tab, message) {
    // Every frame requires the proof marker before any processing — `ready` alone (with
    // or without a client-claimed `auth` field) never grants or restores it. This also
    // covers any unmarked attachment that escaped the constructor's close (defense in
    // depth): the socket is closed and its pending calls failed, not promoted.
    if (tab.deserializeAttachment()?.authenticatedProtocol !== EXEC_PROTOCOL) {
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
      // Keep the proof marker: `ready` only flips readiness and bumps the order.
      tab.serializeAttachment({ authenticatedProtocol: EXEC_PROTOCOL, ready: true, order: ++this.order });
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
    this.finish(reply.id, Response.json(decorated, { status }));
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
