const ROOM = /^rm_[0-9a-f]{64}$/;
const REQUEST_LIMIT = 1024 * 1024;
// The tab budgets its own serialized replies at 7.5 MiB (REPLY_BUDGET in
// apps/figstudio/src/mcp_wire.rs), so a well-formed reply stays under this cap.
const REPLY_LIMIT = 8 * 1024 * 1024;
const encoder = new TextEncoder();
const owns = (body, key) => Object.hasOwn(body, key);
const object = (body) => body !== null && typeof body === "object" && !Array.isArray(body);
const validId = (id) => id === null || typeof id === "string" || typeof id === "number";

function rpcError(status, id, code, message) {
  return Response.json({ jsonrpc: "2.0", id, error: { code, message } }, { status });
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
    // No storage: an eviction drops in-flight requests; the 30 s deadline bounds them.
    this.order = Math.max(0, ...ctx.getWebSockets("tab").map((ws) =>
      ws.deserializeAttachment()?.order ?? 0));
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  activeTab() {
    return this.ctx.getWebSockets("tab")
      .filter((ws) => ws.readyState === 1)
      .sort((a, b) => (b.deserializeAttachment()?.order ?? 0) -
        (a.deserializeAttachment()?.order ?? 0))[0];
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
    const tab = this.activeTab();
    if (!tab) return rpcError(503, body.id, -32000, "No browser tab connected");
    if (this.pending.size >= 128) {
      return rpcError(503, body.id, -32000, "Relay is busy; retry later");
    }
    const id = crypto.randomUUID();
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.finish(id, rpcError(504, body.id, -32000, "Browser tab timed out"));
      }, 30_000);
      this.pending.set(id, { tab, rpcId: body.id, timer, resolve });
      try {
        tab.send(JSON.stringify({ type: "rpc", id, body }));
      } catch {
        this.failTab(tab);
      }
    });
  }

  connect(request) {
    if (request.method !== "GET" || request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("Use a WebSocket upgrade", { status: 426 });
    }
    if (this.ctx.getWebSockets("tab").filter((ws) => ws.readyState === 1).length >= 16) {
      return new Response("Too many browser tabs connected", { status: 503 });
    }
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server, ["tab"]);
    server.serializeAttachment({ order: ++this.order });
    return new Response(null, { status: 101, webSocket: client });
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

  webSocketMessage(tab, message) {
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
      tab.serializeAttachment({ order: ++this.order });
      return;
    }
    if (reply.type !== "rpc_reply") return;
    const entry = this.pending.get(reply.id);
    if (!entry || entry.tab !== tab) return;
    // The reply contract lives in rpc_reply (apps/figstudio/src/mcp_wire.rs): status is
    // always present -- 200 with a JSON-RPC object body, or 202 with a null body.
    const status = reply.status ?? 200; // Lane 2 may omit status on a successful reply.
    if (!Number.isInteger(status) || status < 200 || status > 599 || status === 204 ||
        status === 205 || status === 304 || !object(reply.body) ||
        reply.body.jsonrpc !== "2.0" || reply.body.id !== entry.rpcId ||
        (owns(reply.body, "result") === owns(reply.body, "error"))) {
      this.finish(reply.id, rpcError(502, entry.rpcId, -32000, "Invalid browser reply"));
      return;
    }
    this.finish(reply.id, Response.json(reply.body, { status }));
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
