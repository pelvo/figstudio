import { webcrypto } from "node:crypto";
import assert from "node:assert/strict";
import { McpRoom } from "../worker.mjs";
import { deriveRoom, EXEC_PROTOCOL } from "../executor-auth.mjs";

if (!globalThis.crypto) globalThis.crypto = webcrypto;
globalThis.WebSocketPair = class {};
globalThis.WebSocketRequestResponsePair = class {
  constructor(request, response) { Object.assign(this, { request, response }); }
};

export const roomToken = `rm_${"a".repeat(64)}`;
export const endpoint = `https://relay.example/mcp/${roomToken}`;
// The bridge's shared vector (see executor-auth.test.mjs / mcp_pair_tests.rs): the room
// a real executor derives, so bridge tests prove the subprotocol proof, not a bare room.
export const executorHex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
export const bridgeRoom = await deriveRoom(executorHex);
export const bridgeUrl = (room = bridgeRoom) => `https://relay.example/bridge/${room}`;
export const upgrade = (room = bridgeRoom, headers = {}) => new Request(bridgeUrl(room), {
  method: "GET",
  headers: {
    Upgrade: "websocket",
    Origin: "https://relay.example",
    "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${executorHex}`,
    ...headers,
  },
});
export const rpc = { jsonrpc: "2.0", id: 7, method: "initialize", params: {} };
export const post = (body = rpc) => new Request(endpoint, {
  method: "POST", body: JSON.stringify(body), headers: { "Content-Type": "application/json" },
});

export class Tab {
  constructor(order = 1) {
    // A socket this deploy authenticated and that already said `ready` (the shape
    // `webSocketMessage` stores after the `connect` proof); tests needing other shapes
    // (pre-ready, legacy) set `attachment` explicitly.
    this.attachment = { authenticatedProtocol: EXEC_PROTOCOL, ready: true, order };
    this.readyState = 1;
    this.sent = [];
  }
  serializeAttachment(value) { this.attachment = value; }
  deserializeAttachment() { return this.attachment; }
  send(message) {
    this.sent.push(message);
    this.notify?.(JSON.parse(message));
  }
  close(code) { this.readyState = 3; this.closeCode = code; }
}

export function setup(tabs = [new Tab()]) {
  const ctx = {
    getWebSockets(tag) { assert.equal(tag, "tab"); return tabs; },
    acceptWebSocket(ws, tags) { assert.deepEqual(tags, ["tab"]); tabs.push(ws); },
    setWebSocketAutoResponse(pair) { this.pair = pair; },
    get storage() { throw new Error("Relay must not use storage"); },
  };
  return { room: new McpRoom(ctx), ctx, tabs };
}

export async function forward(room, tab, body = rpc) {
  const sent = new Promise((resolve) => { tab.notify = resolve; });
  const response = room.fetch(post(body));
  const frame = await sent;
  return { response, frame };
}

export function reply(room, tab, frame, extra = {}) {
  room.webSocketMessage(tab, JSON.stringify({
    type: "rpc_reply", id: frame.id, status: 200,
    body: { jsonrpc: "2.0", id: frame.body.id, result: { ok: true } }, ...extra,
  }));
}

export async function errorResponse(response, status, id) {
  assert.equal(response.status, status);
  const body = await response.json();
  assert.equal(body.jsonrpc, "2.0");
  assert.equal(body.id, id);
  assert.equal(typeof body.error.code, "number");
  assert.equal(typeof body.error.message, "string");
  return body;
}
