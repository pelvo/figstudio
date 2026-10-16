import assert from "node:assert/strict";
import { test } from "node:test";
import worker from "../worker.mjs";
import { roomToken, endpoint, rpc, post, Tab, setup, forward, reply, errorResponse,
  upgrade, bridgeRoom, bridgeUrl, executorHex } from "./test-support.mjs";
import { EXEC_PROTOCOL } from "../executor-auth.mjs";

test("rooms are validated before DO lookup, and valid rooms are isolated", async () => {
  const seen = [];
  const env = { MCP_ROOMS: {
    idFromName(name) { seen.push(name); return name; },
    get(id) { return { fetch: () => new Response(id) }; },
  } };
  for (const path of ["/mcp", "/bridge/", "/mcp/rm_short", `/bridge/${bridgeRoom.toUpperCase()}`,
    `/mcp/${bridgeRoom}/extra`, `/mcp/${bridgeRoom}%2f`]) {
    assert.equal((await worker.fetch(new Request(`https://relay.example${path}`), env)).status, 404);
  }
  assert.deepEqual(seen, []);
  for (const token of [roomToken, `rm_${"b".repeat(64)}`]) {
    assert.equal(await (await worker.fetch(new Request(`https://relay.example/mcp/${token}`), env)).text(), token);
  }
  assert.deepEqual(seen, [roomToken, `rm_${"b".repeat(64)}`]);
});

test("the bridge names a Durable Object only behind a valid executor proof", async () => {
  const seen = [];
  const env = { MCP_ROOMS: {
    idFromName(name) { seen.push(name); return name; },
    get(id) { return { fetch: () => new Response(id) }; },
  } };
  // Caller-only possession of the room (the MCP URL) is refused before any Durable
  // Object is named: no proof, a forged Origin alone, a marker without a secret, or a
  // well-formed secret that derives to a different room.
  for (const request of [
    upgrade(bridgeRoom, { Origin: "" }),
    upgrade(bridgeRoom, { "Sec-WebSocket-Protocol": "" }),
    upgrade(bridgeRoom, { "Sec-WebSocket-Protocol": EXEC_PROTOCOL }),
    upgrade(bridgeRoom, { "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${"c".repeat(64)}` }),
    upgrade(roomToken),
  ]) {
    const response = await worker.fetch(request, env);
    assert.equal(response.status, 403, request.url);
    const message = await response.clone().text();
    assert.ok(!message.includes(executorHex), "the refusal must not echo the executor");
  }
  assert.deepEqual(seen, []);
  // The correct proof reaches the Durable Object named by the derived room.
  const response = await worker.fetch(upgrade(), env);
  assert.equal(response.status, 200);
  assert.equal(await response.text(), bridgeRoom);
  assert.deepEqual(seen, [bridgeRoom]);
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

test("an eviction drops in-flight calls: the revived isolate ignores the late reply", async (t) => {
  // Hard eviction: the isolate — its `pending` map and timers — is gone, and the same
  // sockets are reattached to a fresh constructor. The caller's connection died with
  // the isolate; the late `rpc_reply` finds no pending entry and must not revive it.
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const tab = new Tab(1);
  const { room } = setup([tab]);
  const { frame, response } = await forward(room, tab);
  let settled = false;
  response.then(() => { settled = true; });
  const { room: revived } = setup([tab]);
  reply(revived, tab, frame);
  await Promise.resolve();
  assert.equal(settled, false, "a reply to a new isolate cannot resolve the old call");
  // In a real eviction even this timer is cancelled with the isolate; firing it here
  // shows the caller never gets a resolution from the reply alone.
  t.mock.timers.tick(30_000);
  await errorResponse(await response, 504, 7);
  // The revived isolate still serves new calls on the same socket.
  const fresh = await forward(revived, tab, { ...rpc, id: 11 });
  reply(revived, tab, fresh.frame);
  assert.equal((await fresh.response).status, 200);
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
    if (init?.status === 101) return { status: 101, webSocket: init.webSocket, headers: init.headers };
    return new NativeResponse(body, init);
  });
  const response = await room.connect(upgrade());
  assert.equal(response.status, 101);
  // The 101 echoes only the constant marker, never the offered executor hex.
  assert.deepEqual(response.headers, { "Sec-WebSocket-Protocol": EXEC_PROTOCOL });
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

test("a socket stored by an earlier deploy is closed at restore and never regains the role", async () => {
  // Attachments written before the authenticated marker existed: `{order}` (connected,
  // never ready) and `{ready:true, order}` (accepted by a deploy where the room alone
  // admitted the bridge — the old-socket hole). Restoring them closes both; a re-sent
  // `ready` — with or without a client-claimed marker — promotes neither.
  const legacy = [{ order: 9 }, { ready: true, order: 9 }].map((attachment, i) => {
    const tab = new Tab(9 + i);
    tab.attachment = attachment;
    return tab;
  });
  const { room } = setup(legacy);
  for (const tab of legacy) {
    assert.equal(tab.readyState, 3, "the constructor closes a legacy socket");
    assert.equal(tab.closeCode, 1008);
  }
  room.webSocketMessage(legacy[1], JSON.stringify({
    type: "ready", protocol: 1, authenticatedProtocol: EXEC_PROTOCOL, auth: EXEC_PROTOCOL,
  }));
  await errorResponse(await room.fetch(post()), 503, 7);
});

test("an open socket without the authenticated marker never takes calls", async () => {
  // Marker lost on an open socket (an earlier-deploy attachment, or a failed attachment
  // write at accept): it looks newest-and-ready but must never be the executor.
  const live = new Tab(1);
  const { room, tabs } = setup([live]);
  const unmarked = new Tab(3);
  unmarked.attachment = { ready: true, order: 3 };
  tabs.push(unmarked);
  const delivered = new Promise((resolve) => { live.notify = resolve; });
  const response = room.fetch(post());
  const frame = await Promise.race([
    delivered,
    new Promise((resolve) => setTimeout(() => resolve(null), 250)),
  ]);
  assert.ok(frame, "the call must reach the authenticated tab, not the unmarked socket");
  reply(room, live, frame);
  assert.equal((await response).status, 200);
});

test("an unmarked socket's rpc_reply steals nothing", async () => {
  const live = new Tab(1);
  const { room, tabs } = setup([live]);
  const unmarked = new Tab(3);
  unmarked.attachment = { ready: true, order: 3 };
  tabs.push(unmarked);
  const delivered = new Promise((resolve) => { live.notify = resolve; });
  const response = room.fetch(post());
  const frame = await Promise.race([
    delivered,
    new Promise((resolve) => setTimeout(() => resolve(null), 250)),
  ]);
  assert.ok(frame, "the call must reach the authenticated tab");
  let settled = false;
  response.then(() => { settled = true; });
  room.webSocketMessage(unmarked, JSON.stringify({
    type: "rpc_reply", id: frame.id, status: 200,
    body: { jsonrpc: "2.0", id: 7, result: { stolen: true } },
  }));
  await Promise.resolve();
  assert.equal(settled, false, "an unmarked socket must not resolve a pending call");
  assert.equal(unmarked.closeCode, 1008);
  reply(room, live, frame);
  assert.deepEqual(await (await response).json(), { jsonrpc: "2.0", id: 7, result: { ok: true } });
});

test("an unmarked socket is closed on its first frame and never promoted", async () => {
  const { room, tabs } = setup([]);
  const unmarked = new Tab(3);
  unmarked.attachment = { ready: true, order: 3 };
  tabs.push(unmarked);
  room.webSocketMessage(unmarked, JSON.stringify({
    type: "ready", protocol: 1, authenticatedProtocol: EXEC_PROTOCOL, auth: EXEC_PROTOCOL,
  }));
  assert.equal(unmarked.closeCode, 1008);
  assert.equal(unmarked.attachment.ready, true, "`ready` must not be (re)granted");
  await errorResponse(await room.fetch(post()), 503, 7);
});

test("an unmarked socket fails pending calls even when close throws", async () => {
  const tab = new Tab(1);
  const { room } = setup([tab]);
  const pending = await forward(room, tab);
  // Lose the marker after a call was dispatched: rejection must fail that call,
  // even if the runtime refuses to close the restored socket.
  const attachment = { ready: false, order: 1 };
  tab.attachment = attachment;
  tab.close = () => { throw new Error("already closing"); };
  assert.doesNotThrow(() => room.webSocketMessage(tab, JSON.stringify({
    type: "ready", protocol: 1, authenticatedProtocol: EXEC_PROTOCOL,
  })));
  await errorResponse(await pending.response, 503, 7);
  assert.deepEqual(tab.attachment, { ready: false, order: 1 });
  assert.equal(room.pending.size, 0);
  await errorResponse(await room.fetch(post()), 503, 7);
  assert.doesNotThrow(() => reply(room, tab, pending.frame));
});

test("an authenticated socket without a ready attachment waits for ready", async () => {
  const unready = new Tab(1);
  unready.attachment = { authenticatedProtocol: EXEC_PROTOCOL, ready: false, order: 1 };
  const { room } = setup([unready]);
  await errorResponse(await room.fetch(post()), 503, 7);
  room.webSocketMessage(unready, JSON.stringify({ type: "ready", protocol: 1 }));
  assert.deepEqual(unready.attachment, { authenticatedProtocol: EXEC_PROTOCOL, ready: true, order: 2 });
  const { frame, response } = await forward(room, unready);
  reply(room, unready, frame);
  assert.equal((await response).status, 200);
});

test("an accepted socket proves the executor again at the Durable Object", async () => {
  // The outer Worker already checked the proof; the Durable Object must not trust that
  // on anyone's say-so (a forgeable header), so it re-verifies before accepting.
  const { room } = setup([]);
  for (const request of [
    upgrade(bridgeRoom, { Origin: "https://foreign.example" }),
    upgrade(bridgeRoom, { "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${"c".repeat(64)}` }),
  ]) {
    const response = await room.fetch(request);
    assert.equal(response.status, 403, request.url);
  }
  assert.equal((await room.fetch(new Request(bridgeUrl()))).status, 426);
});
