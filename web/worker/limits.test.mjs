import assert from "node:assert/strict";
import { test } from "node:test";
import { endpoint, rpc, post, setup, forward, reply, errorResponse } from "./test-support.mjs";

test("bad JSON, batches and malformed messages are rejected before forwarding", async () => {
  const { room, tabs: [tab] } = setup();
  const malformed = new Request(endpoint, { method: "POST", body: "{" });
  assert.equal((await errorResponse(await room.fetch(malformed), 400, null)).error.code, -32700);
  for (const body of [[rpc], null, 5, {}, { ...rpc, id: {} }, { ...rpc, method: 5 },
    { ...rpc, result: {} }, { jsonrpc: "2.0", id: 1 }]) {
    await errorResponse(await room.fetch(post(body)), 400, typeof body?.id === "number" ? body.id : null);
  }
  assert.equal(tab.sent.length, 0);
});

test("request limit is enforced for streamed bytes, not only content-length", async () => {
  const { room } = setup([]);
  const data = new TextEncoder().encode("x".repeat(600_000));
  let cancelled = false;
  const stream = new ReadableStream({
    pull(controller) { controller.enqueue(data); },
    cancel() { cancelled = true; },
  });
  await errorResponse(await room.fetch(new Request(endpoint, {
    method: "POST", body: stream, duplex: "half",
  })), 413, null);
  assert.equal(cancelled, true);
  await errorResponse(await room.fetch(new Request(endpoint, {
    method: "POST", body: "{}", headers: { "Content-Length": "1048577" },
  })), 413, null);
});

test("invalid and oversized replies fail the caller rather than leaking an unrelated body", async () => {
  for (const extra of [{ status: 101 }, { status: 204 }, { body: [] },
    { body: { jsonrpc: "2.0", id: 8, result: {} } },
    { body: { jsonrpc: "2.0", id: 7, result: {}, error: {} } }]) {
    const { room, tabs: [tab] } = setup();
    const { frame, response } = await forward(room, tab);
    reply(room, tab, frame, extra);
    await errorResponse(await response, 502, 7);
  }
  for (const message of [new ArrayBuffer(8), "x".repeat(8 * 1024 * 1024 + 1)]) {
    const { room, tabs: [tab] } = setup();
    const { response } = await forward(room, tab);
    room.webSocketMessage(tab, message);
    await errorResponse(await response, 503, 7);
    assert.equal(tab.closeCode, 1009);
  }
});

test("an oversized rpc_reply fails only its call and keeps the socket", async () => {
  const { room, tabs: [tab] } = setup();
  const first = await forward(room, tab);
  const second = await forward(room, tab, { ...rpc, id: 8 });
  room.webSocketMessage(tab, JSON.stringify({
    type: "rpc_reply", id: first.frame.id, status: 200,
    body: { jsonrpc: "2.0", id: 7, result: { content: [{ type: "text", text: "界".repeat(3 * 1024 * 1024) }] } },
  }));
  await errorResponse(await first.response, 502, 7);
  assert.equal(tab.readyState, 1, "the tab socket must stay open");
  assert.equal(tab.closeCode, undefined);
  reply(room, tab, second.frame);
  assert.equal((await second.response).status, 200);
});

test("status-less Lane 2 replies default to JSON mode HTTP 200", async () => {
  const { room, tabs: [tab] } = setup();
  const { frame, response } = await forward(room, tab);
  reply(room, tab, frame, { status: undefined });
  assert.equal((await response).status, 200);
});

test("a failed send promptly fails the call", async () => {
  const { room, tabs: [tab] } = setup();
  tab.send = () => { throw new Error("closed"); };
  await errorResponse(await room.fetch(post()), 503, 7);
});

test("bridge requires an upgrade and refuses a foreign browser Origin", async () => {
  const { room } = setup([]);
  const url = endpoint.replace("/mcp/", "/bridge/");
  assert.equal((await room.fetch(new Request(url))).status, 426);
  assert.equal((await room.fetch(new Request(url, {
    headers: { Upgrade: "websocket", Origin: "https://foreign.example" },
  }))).status, 403);
});

test("concurrent requests and connected tabs have bounded admission", async () => {
  const { room, tabs: [tab] } = setup();
  const calls = [];
  for (let id = 0; id < 128; id++) {
    calls.push(await forward(room, tab, { ...rpc, id }));
  }
  await errorResponse(await room.fetch(post()), 503, 7);
  for (const call of calls) reply(room, tab, call.frame);
  assert.equal((await Promise.all(calls.map((call) => call.response))).length, 128);
  const { room: full } = setup(Array.from({ length: 16 }, () => new tab.constructor()));
  assert.equal((await full.fetch(new Request(endpoint.replace("/mcp/", "/bridge/"), {
    headers: { Upgrade: "websocket" },
  }))).status, 503);
});

test("MCP refuses a foreign browser Origin but accepts same-origin clients", async () => {
  const { room } = setup([]);
  for (const origin of ["null", "https://foreign.example"]) {
    const request = post();
    request.headers.set("Origin", origin);
    assert.equal((await room.fetch(request)).status, 403);
  }
  const request = post();
  request.headers.set("Origin", new URL(endpoint).origin);
  await errorResponse(await room.fetch(request), 503, 7);
});

test("upgrade accepts a tagged hibernating socket and makes it the newest executor", async (t) => {
  const { room, ctx, tabs } = setup();
  const server = new tabs[0].constructor(0);
  const client = {};
  t.mock.method(globalThis, "WebSocketPair", function () { return { 0: client, 1: server }; });
  const NativeResponse = globalThis.Response;
  t.mock.method(globalThis, "Response", function (body, init) {
    if (init?.status === 101) return { status: 101, webSocket: init.webSocket };
    return new NativeResponse(body, init);
  });
  const response = room.connect(new Request(endpoint.replace("/mcp/", "/bridge/"), {
    headers: { Upgrade: "websocket", Origin: new URL(endpoint).origin },
  }));
  assert.equal(response.status, 101);
  assert.equal(response.webSocket, client);
  assert.equal(ctx.getWebSockets("tab").at(-1), server);
  t.mock.restoreAll();
  const { frame, response: rpcResponse } = await forward(room, server);
  reply(room, server, frame);
  assert.equal((await rpcResponse).status, 200);
});
