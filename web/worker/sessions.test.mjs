import assert from "node:assert/strict";
import { test } from "node:test";
import { deriveSessionId, RELAY_TOOLS, sha256Hex } from "../worker.mjs";
import { endpoint, post, rpc, Tab, setup, forward, reply } from "./test-support.mjs";

function readyV2(deviceId, label, instance, { focused = false, docs = [] } = {}) {
  return JSON.stringify({
    type: "ready",
    protocol: 2,
    device: { id: deviceId, label },
    instance,
    focused,
    docs,
  });
}

function callTool(name, args = {}, id = 1) {
  return {
    jsonrpc: "2.0",
    id,
    method: "tools/call",
    params: { name, arguments: args },
  };
}

async function initClient(room, id = 100) {
  const tab = room.activeTab();
  const { frame, response } = await forward(room, tab, { ...rpc, id });
  reply(room, tab, frame, {
    body: { jsonrpc: "2.0", id, result: { protocolVersion: "2025-03-26", capabilities: {} } },
  });
  const res = await response;
  const mcpSessionId = res.headers.get("Mcp-Session-Id");
  assert.ok(mcpSessionId, "initialize must issue an Mcp-Session-Id header");
  return mcpSessionId;
}

test("sha256Hex matches standard NIST vector and ready v2 stores sanitized, bounded metadata", async () => {
  assert.equal(
    sha256Hex("abc"),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
  );
  const tab = new Tab(0);
  const { room } = setup([tab]);
  const longFileKey = `${"A".repeat(350)}~7`;
  const manyDocs = [
    ...Array.from({ length: 22 }, (_, i) => ({
      tab: i,
      name: `${"A".repeat(95)}😀\n\u202efig`,
      fileKey: i === 0 ? longFileKey : `${"k".repeat(350)}_${i}`,
      key: "0123456789abcdef0123456789abcdef_extra",
      pages: i + 1,
    })),
    {
      tab: 22,
      name: "😀".repeat(96),
      fileKey: "X".repeat(4_000),
      key: "0123456789abcdef",
      pages: 1,
    },
    {
      tab: 23,
      name: "after.fig",
      fileKey: "after-key",
      key: "fedcba9876543210",
      pages: 2,
    },
  ];
  room.webSocketMessage(tab, readyV2("dev_office", "Office\n\u202e · Chrome", "inst_1", {
    focused: true,
    docs: manyDocs,
  }));
  const att = tab.deserializeAttachment();
  assert.equal(att.ready, true);
  assert.equal(att.sessionId, deriveSessionId("dev_office", "inst_1"));
  assert.match(att.sessionId, /^s_[0-9a-f]{32}$/);
  assert.equal(att.deviceId, "dev_office");
  assert.equal(att.deviceLabel, "Office  · Chrome");
  assert.ok(!att.deviceLabel.includes("\n") && !att.deviceLabel.includes("\u202e"));
  assert.equal(att.focused, true);
  assert.equal(att.docs.length, 23, "oversized doc 22 is skipped so doc 23 ('after.fig') still fits in the 12 KiB budget");
  assert.equal(Array.from(att.docs[0].name).length, 96);
  assert.equal(att.docs[0].name.includes("😀"), true, "code-point slice must not split surrogate pairs");
  assert.ok(!att.docs[0].name.includes("\n") && !att.docs[0].name.includes("\u202e"));
  assert.equal(att.docs[0].fileKey, longFileKey, "fileKey must round-trip verbatim without truncation");
  assert.equal(att.docs[22].name, "after.fig");
  assert.equal(att.docs[22].fileKey, "after-key");
  assert.equal(att.docs[0].key.length, 32);
  assert.ok(JSON.stringify(att).length < 16_384, "attachment must stay well under the 16 KiB DO limit");

  // list_sessions must never expose `device.id`
  const listRes = await room.fetch(post(callTool("list_sessions", {}, 2)));
  const listBody = await listRes.json();
  const s0 = listBody.result.structuredContent.sessions[0];
  assert.equal(s0.session, att.sessionId);
  assert.deepEqual(s0.device, { label: "Office  · Chrome" });
  assert.equal("id" in s0.device, false, "device.id must not be published");

  // When the budget is NOT crowded, a doc whose `fileKey` alone exceeds 12 KiB is stored
  // without `fileKey` (preserving its `name` and `key`) alongside subsequent docs.
  room.webSocketMessage(tab, readyV2("dev_office", "Office · Chrome", "inst_1", {
    focused: true,
    docs: [
      { tab: 1, name: "wide.fig", fileKey: "W".repeat(20_000), key: "0123456789abcdef", pages: 3 },
      { tab: 2, name: "after.fig", fileKey: "after-key", key: "fedcba9876543210", pages: 1 },
    ],
  }));
  const sparseAtt = tab.deserializeAttachment();
  assert.deepEqual(sparseAtt.docs, [
    { tab: 1, name: "wide.fig", key: "0123456789abcdef", pages: 3 },
    { tab: 2, name: "after.fig", fileKey: "after-key", key: "fedcba9876543210", pages: 1 },
  ]);
});

test("tools/list works with no tab connected, augments tab tools, and preserves native session args", async () => {
  const { room: emptyRoom } = setup([]);
  const emptyRes = await emptyRoom.fetch(post({ jsonrpc: "2.0", id: 1, method: "tools/list" }));
  assert.equal(emptyRes.status, 200);
  const emptyBody = await emptyRes.json();
  assert.deepEqual(
    emptyBody.result.tools.map((t) => t.name),
    RELAY_TOOLS.map((t) => t.name),
  );

  const tab = new Tab(1);
  const { room } = setup([tab]);
  room.webSocketMessage(tab, readyV2("dev_1", "Mac", "inst_1"));
  const { frame, response } = await forward(room, tab, { jsonrpc: "2.0", id: 2, method: "tools/list" });
  reply(room, tab, frame, {
    body: {
      jsonrpc: "2.0",
      id: 2,
      result: {
        tools: [
          {
            name: "get_metadata",
            inputSchema: { type: "object", properties: { tab: { type: "integer" } } },
          },
          {
            name: "custom_with_session",
            inputSchema: { type: "object", properties: { session: { type: "string", description: "native" } } },
          },
        ],
      },
    },
  });
  const listed = await (await response).json();
  const names = listed.result.tools.map((t) => t.name);
  assert.deepEqual(names, ["get_metadata", "custom_with_session", "list_sessions", "select_session"]);
  assert.equal(listed.result.tools[0].inputSchema.properties.session.type, "string");
  assert.equal(listed.result.tools[1].inputSchema.properties.session.description, "native");

  // Calling `custom_with_session` preserves its native `session` argument instead of stripping it.
  const sent = new Promise((resolve) => { tab.notify = resolve; });
  const callPromise = room.fetch(post(callTool("custom_with_session", { session: "native-val", x: 1 }, 3)));
  const callFrame = await sent;
  assert.deepEqual(callFrame.body.params.arguments, { session: "native-val", x: 1 });
  reply(room, tab, callFrame, {
    body: { jsonrpc: "2.0", id: 3, result: { content: [] } },
  });
  await callPromise;
});

test("single device with multiple tabs routes to newest focused tab even when labels differ on first ready", async () => {
  const tabA = new Tab(1);
  const tabB = new Tab(2);
  const { room } = setup([tabA, tabB]);
  // `deviceLabel` is display-only: two sockets sharing `deviceId` with different labels
  // (including a socket whose first `ready` carries another label) route to the newest focused tab.
  room.webSocketMessage(tabA, readyV2("dev_same", "Office MBP", "tab_a", { focused: true }));
  room.webSocketMessage(tabB, readyV2("dev_same", "Renamed MBP", "tab_b", { focused: true }));
  const sentB = new Promise((resolve) => { tabB.notify = resolve; });
  const resPromise = room.fetch(post(callTool("get_metadata", { node: "0:1" }, 10)));
  const frameB = await sentB;
  reply(room, tabB, frameB, {
    body: { jsonrpc: "2.0", id: 10, result: { content: [{ type: "text", text: "from B" }] } },
  });
  const bodyB = await (await resPromise).json();
  assert.equal(bodyB.result._meta["figstudio/session"], deriveSessionId("dev_same", "tab_b"));
  assert.equal(bodyB.result._meta["figstudio/device"], "Renamed MBP");
});

test("multiple devices without a session argument return -32002 without leaking device.id", async () => {
  const office = new Tab(1);
  const home = new Tab(2);
  const { room } = setup([office, home]);
  room.webSocketMessage(office, readyV2("dev_office", "Office Mac", "inst_o", {
    docs: [{ tab: 1, name: "design.fig", key: "abcd1234", pages: 3 }],
  }));
  room.webSocketMessage(home, readyV2("dev_home", "Home PC", "inst_h", {
    docs: [{ tab: 1, name: "home.fig", key: "ef009988", pages: 1 }],
  }));

  const res = await room.fetch(post(callTool("get_metadata", {}, 20)));
  assert.equal(res.status, 200);
  const body = await res.json();
  assert.equal(body.error.code, -32002);
  assert.equal(body.error.data.sessions.length, 2);
  for (const s of body.error.data.sessions) {
    assert.equal("id" in s.device, false, "error.data must not leak device.id");
  }
  assert.equal(office.sent.length, 0);
  assert.equal(home.sent.length, 0);

  // Explicit `arguments.session` routes to the chosen device and strips `session` before forwarding.
  const officeSession = deriveSessionId("dev_office", "inst_o");
  const sentOffice = new Promise((resolve) => { office.notify = resolve; });
  const targetedPromise = room.fetch(post(callTool("get_metadata", { session: officeSession, tab: 1 }, 21)));
  const frame = await sentOffice;
  assert.deepEqual(frame.body.params.arguments, { tab: 1 });
  reply(room, office, frame, {
    body: { jsonrpc: "2.0", id: 21, result: { content: [{ type: "text", text: "office ok" }] } },
  });
  const targetedBody = await (await targetedPromise).json();
  assert.equal(targetedBody.result._meta["figstudio/session"], officeSession);
});

test("two protocol-1 sockets get distinct device keys (-32002), keep stable sessionId across ready, and tag _meta", async () => {
  const legacyA = new Tab(1);
  const legacyB = new Tab(2);
  const { room } = setup([legacyA, legacyB]);
  const v1Ready = JSON.stringify({ type: "ready", protocol: 1 });
  room.webSocketMessage(legacyA, v1Ready);
  room.webSocketMessage(legacyB, v1Ready);

  // Two protocol-1 sockets must NOT collapse to a single empty deviceId in Rule 3.
  const ambig = await (await room.fetch(post(callTool("get_metadata", {}, 25)))).json();
  assert.equal(ambig.error.code, -32002);
  assert.equal(ambig.error.data.sessions.length, 2);
  const sessionA = ambig.error.data.sessions.find((s) => s.session === deriveSessionId("v1", "1")).session;
  assert.ok(sessionA);

  // Pin legacyA via initialize + select_session, then re-send protocol-1 ready on legacyA:
  // its sessionId must stay stable so the pin is not dropped, and _meta is populated.
  const mcpSession = await initClient(room, 26);
  const postWithMcp = (b) => new Request(endpoint, {
    method: "POST",
    body: JSON.stringify(b),
    headers: { "Content-Type": "application/json", "Mcp-Session-Id": mcpSession },
  });
  const sel = await (await room.fetch(postWithMcp(callTool("select_session", { session: sessionA }, 27)))).json();
  assert.equal(sel.result.isError, undefined);

  // legacyA sends ready again ( bumps order, keeps sessionId )
  room.webSocketMessage(legacyA, v1Ready);
  const sentA = new Promise((resolve) => { legacyA.notify = resolve; });
  const callPromise = room.fetch(postWithMcp(callTool("get_metadata", {}, 28)));
  const frameA = await sentA;
  reply(room, legacyA, frameA, {
    body: { jsonrpc: "2.0", id: 28, result: { content: [{ type: "text", text: "from v1 A" }] } },
  });
  const callBody = await (await callPromise).json();
  assert.equal(callBody.result._meta["figstudio/session"], sessionA);
  assert.equal(callBody.result._meta["figstudio/device"], "Browser tab");
});

test("duplicate sessionId across live sockets fails closed with -32002 on both explicit and auto routing", async () => {
  const orig = new Tab(1);
  const clone = new Tab(2);
  clone.attachment = { ...clone.attachment, ready: false };
  const { room } = setup([orig, clone]);
  // Both sockets claim the exact same (deviceId, instance) -> duplicate sessionId
  room.webSocketMessage(orig, readyV2("dev_office", "Office Mac", "inst_1", { focused: true }));
  room.webSocketMessage(clone, readyV2("dev_office", "Office Mac", "inst_1", { focused: false }));

  const targetSession = deriveSessionId("dev_office", "inst_1");
  // Explicit session must refuse to pick newest when two live sockets share the id
  const explicitRes = await (await room.fetch(post(callTool("get_metadata", { session: targetSession }, 40)))).json();
  assert.equal(explicitRes.error.code, -32002);
  assert.match(explicitRes.error.error?.message ?? explicitRes.error.message, /multiple executors/);

  // Auto routing (Rule 3) also refuses when two live sockets collide on the same sessionId
  const rule3Res = await (await room.fetch(post(callTool("get_metadata", {}, 41)))).json();
  assert.equal(rule3Res.error.code, -32002);
});

test("Mcp-Session-Id is minted on initialize, isolated per client, rejects unrecognized ids with HTTP 404, caps at 64, and fails on dead pin", async () => {
  const office = new Tab(1);
  const home = new Tab(2);
  const { room } = setup([office, home]);
  room.webSocketMessage(office, readyV2("dev_office", "Office Mac", "inst_o"));
  room.webSocketMessage(home, readyV2("dev_home", "Home PC", "inst_h"));

  const officeSession = deriveSessionId("dev_office", "inst_o");
  const homeSession = deriveSessionId("dev_home", "inst_h");

  // Unissued client-supplied Mcp-Session-Id is rejected with HTTP 404 on select_session, tools/call, and tools/list
  const forgedReq = (b) => new Request(endpoint, {
    method: "POST",
    body: JSON.stringify(b),
    headers: { "Content-Type": "application/json", "Mcp-Session-Id": "forged-client-id" },
  });
  for (const reqBody of [
    callTool("select_session", { session: homeSession }, 50),
    callTool("get_metadata", {}, 501),
    { jsonrpc: "2.0", id: 502, method: "tools/list", params: {} },
    { jsonrpc: "2.0", method: "notifications/cancelled", params: { requestId: 1 } },
  ]) {
    const forgedRes = await room.fetch(forgedReq(reqBody));
    assert.equal(forgedRes.status, 404);
    const forgedJson = await forgedRes.json();
    assert.equal(forgedJson.error.code, -32002);
  }
  assert.equal(room.mcpSessions.size, 0);
  assert.equal(office.sent.length, 0);
  assert.equal(home.sent.length, 0);

  // Two independent MCP clients initialize and get distinct server-issued Mcp-Session-Ids
  const mcpClient1 = await initClient(room, 51);
  const mcpClient2 = await initClient(room, 52);
  assert.notEqual(mcpClient1, mcpClient2);

  const req1 = (b) => new Request(endpoint, {
    method: "POST",
    body: JSON.stringify(b),
    headers: { "Content-Type": "application/json", "Mcp-Session-Id": mcpClient1 },
  });
  const req2 = (b) => new Request(endpoint, {
    method: "POST",
    body: JSON.stringify(b),
    headers: { "Content-Type": "application/json", "Mcp-Session-Id": mcpClient2 },
  });

  // Client 1 pins home; Client 2 stays unpinned (gets -32002)
  const sel1 = await room.fetch(req1(callTool("select_session", { session: homeSession }, 53)));
  assert.equal(sel1.headers.get("Mcp-Session-Id"), mcpClient1);
  assert.equal((await sel1.json()).result.isError, undefined);

  const c2Call = await (await room.fetch(req2(callTool("get_metadata", {}, 54)))).json();
  assert.equal(c2Call.error.code, -32002);

  // Office sends ready again: Client 1 stays pinned to home
  room.webSocketMessage(office, readyV2("dev_office", "Office Mac", "inst_o", { focused: true }));
  const sentHome = new Promise((resolve) => { home.notify = resolve; });
  const c1Promise = room.fetch(req1(callTool("get_metadata", {}, 55)));
  const frameHome = await sentHome;
  reply(room, home, frameHome, {
    body: { jsonrpc: "2.0", id: 55, result: { content: [{ type: "text", text: "still home" }] } },
  });
  const c1Body = await (await c1Promise).json();
  assert.equal(c1Body.result._meta["figstudio/session"], homeSession);

  // Now close the pinned `home` socket while `office` is still ready (only 1 device left!):
  // Client 1 must NOT silently fall through to `office`; it must get -32002 with live sessions.
  room.webSocketClose(home);
  const afterClose = await (await room.fetch(req1(callTool("get_metadata", {}, 56)))).json();
  assert.equal(afterClose.error.code, -32002);
  assert.equal(afterClose.error.data.sessions.length, 1);
  assert.equal(afterClose.error.data.sessions[0].session, officeSession);
  assert.equal(office.sent.length, 0, "office must not have received the tool call");

  // Cap check: minting > 64 MCP sessions evicts the oldest
  for (let i = 0; i < 70; i++) {
    await initClient(room, 200 + i);
  }
  assert.equal(room.mcpSessions.size, 64);
  assert.equal(room.mcpSessions.has(mcpClient1), false, "oldest MCP session should be evicted past cap 64");

  // Even when only 1 device (`office`) remains open, calling with an evicted/unrecognized
  // `Mcp-Session-Id` must return HTTP 404 (-32002) instead of falling through to Rule 3.
  const sentBefore = office.sent.length;
  const evictedRes = await room.fetch(req1(callTool("get_metadata", {}, 300)));
  assert.equal(evictedRes.status, 404);
  const afterLruEvict = await evictedRes.json();
  assert.equal(afterLruEvict.error.code, -32002);
  assert.equal(office.sent.length, sentBefore, "evicted Mcp-Session-Id must not fall through to Rule 3");
});
