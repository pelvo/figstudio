import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "../worker.mjs";
import { roomToken, endpoint, rpc, post, Tab, setup, forward, reply, errorResponse } from "./test-support.mjs";

test("rooms are validated before DO lookup, and valid rooms are isolated", async () => {
  const seen = [];
  const env = { MCP_ROOMS: {
    idFromName(name) { seen.push(name); return name; },
    get(id) { return { fetch: () => new Response(id) }; },
  } };
  for (const path of ["/mcp", "/bridge/", "/mcp/rm_short", `/bridge/${roomToken.toUpperCase()}`,
    `/mcp/${roomToken}/extra`, `/mcp/${roomToken}%2f`]) {
    assert.equal((await worker.fetch(new Request(`https://relay.example${path}`), env)).status, 404);
  }
  assert.deepEqual(seen, []);
  for (const token of [roomToken, `rm_${"b".repeat(64)}`]) {
    assert.equal(await (await worker.fetch(new Request(`https://relay.example/bridge/${token}`), env)).text(), token);
  }
  assert.equal(seen.length, 2);
});

test("static pass-through keeps isolation headers and original request", async () => {
  const request = new Request("https://relay.example/figstudio_bg.wasm");
  const response = new Response("asset", { headers: {
    "Cross-Origin-Opener-Policy": "same-origin",
    "Cross-Origin-Embedder-Policy": "require-corp",
    "Cross-Origin-Resource-Policy": "same-origin",
    "X-Content-Type-Options": "nosniff",
  } });
  const result = await worker.fetch(request, { ASSETS: { fetch(received) {
    assert.equal(received, request); return response;
  } } });
  assert.equal(result, response);
  assert.equal(result.headers.get("Cross-Origin-Embedder-Policy"), "require-corp");
  assert.equal(result.headers.get("Cross-Origin-Opener-Policy"), "same-origin");
  assert.equal(result.headers.get("Cross-Origin-Resource-Policy"), "same-origin");
});

test("notifications and JSON-RPC responses return 202 without a tab", async () => {
  const { room } = setup([]);
  for (const body of [{ jsonrpc: "2.0", method: "notifications/initialized" },
    { jsonrpc: "2.0", id: 4, result: {} },
    { jsonrpc: "2.0", id: 4, error: { code: -32601, message: "Unknown" } }]) {
    const response = await room.fetch(post(body));
    assert.equal(response.status, 202);
    assert.equal(await response.text(), "");
  }
});

test("RPCs are forwarded unchanged, matched by both socket and envelope id", async () => {
  const { room, tabs: [tab] } = setup();
  const first = await forward(room, tab);
  const second = await forward(room, tab, { ...rpc, id: "second" });
  assert.equal(first.frame.type, "rpc");
  assert.deepEqual(first.frame.body, rpc);
  assert.match(first.frame.id, /^[0-9a-f-]{36}$/);
  assert.notEqual(first.frame.id, second.frame.id);
  reply(room, new Tab(), first.frame, {
    body: { jsonrpc: "2.0", id: 7, result: { foreign: true } },
  });
  reply(room, tab, { ...first.frame, id: "unknown" });
  reply(room, tab, second.frame, { status: 201 });
  assert.equal((await second.response).status, 201);
  reply(room, tab, first.frame);
  assert.deepEqual(await (await first.response).json(), {
    jsonrpc: "2.0", id: 7, result: { ok: true },
  });
});

test("no tab and unsupported SSE return explicit errors", async () => {
  const { room } = setup([]);
  await errorResponse(await room.fetch(post()), 503, 7);
  const response = await room.fetch(new Request(endpoint));
  assert.equal(response.status, 405);
  assert.equal(response.headers.get("Allow"), "POST");
});

test("deadline is 30 seconds and late replies cannot revive a timed-out call", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { room, tabs: [tab] } = setup();
  const { frame, response } = await forward(room, tab);
  let settled = false;
  response.then(() => { settled = true; });
  t.mock.timers.tick(29_999);
  await Promise.resolve();
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  await errorResponse(await response, 504, 7);
  reply(room, tab, frame);
});

test("close and error fail only that socket's pending calls", async () => {
  for (const event of ["webSocketClose", "webSocketError"]) {
    const tab = new Tab(1);
    const other = new Tab(0);
    const { room } = setup([tab, other]);
    const first = await forward(room, tab);
    room.webSocketMessage(other, JSON.stringify({ type: "ready", protocol: 1 }));
    const second = await forward(room, other, { ...rpc, id: 9 });
    room[event](tab);
    await errorResponse(await first.response, 503, 7);
    reply(room, other, second.frame);
    assert.equal((await second.response).status, 200);
  }
});

test("the last tab to say ready takes the calls", async () => {
  const first = new Tab(1);
  const second = new Tab(1);
  const { room } = setup([first, second]);
  const ready = JSON.stringify({ type: "ready", protocol: 1 });
  room.webSocketMessage(first, ready);
  const toFirst = await forward(room, first);
  reply(room, first, toFirst.frame);
  await toFirst.response;
  // The second tab readied later: it now handles calls, the first one no longer does.
  room.webSocketMessage(second, ready);
  const delivered = new Promise((resolve) => { second.notify = resolve; });
  const response = room.fetch(post());
  const winner = await Promise.race([
    delivered.then(() => "second"),
    new Promise((resolve) => setTimeout(() => resolve("nobody"), 250)),
  ]);
  assert.equal(winner, "second", "the call must go to the newest ready tab");
  assert.equal(first.sent.length, 1, "the first tab must not receive new calls");
  const frame = await delivered;
  reply(room, second, frame);
  assert.equal((await response).status, 200);
});

test("the 16-tab cap counts only open sockets", async (t) => {
  // Half the sockets are mid-close (readyState 0): they do not count, as with activeTab.
  const tabs = Array.from({ length: 16 }, (_, i) => {
    const tab = new Tab();
    tab.readyState = i % 2;
    return tab;
  });
  const { room } = setup(tabs);
  const server = new tabs[0].constructor(0);
  const client = {};
  t.mock.method(globalThis, "WebSocketPair", function () { return { 0: client, 1: server }; });
  const NativeResponse = globalThis.Response;
  t.mock.method(globalThis, "Response", function (body, init) {
    if (init?.status === 101) return { status: 101, webSocket: init.webSocket };
    return new NativeResponse(body, init);
  });
  const response = room.connect(new Request(endpoint.replace("/mcp/", "/bridge/"), {
    headers: { Upgrade: "websocket" },
  }));
  assert.equal(response.status, 101);
  t.mock.restoreAll();
});

test("hibernated socket attachments preserve active order and ping auto-response", async () => {
  const older = new Tab(2);
  const newer = new Tab(5);
  const { room, ctx } = setup([older, newer]);
  assert.deepEqual({ ...ctx.pair }, { request: "ping", response: "pong" });
  const first = await forward(room, newer);
  reply(room, newer, first.frame);
  await first.response;
  room.webSocketMessage(older, JSON.stringify({ type: "ready", protocol: 1 }));
  const { room: revived } = setup([older, newer]);
  const second = await forward(revived, older);
  reply(revived, older, second.frame);
  await second.response;
});
