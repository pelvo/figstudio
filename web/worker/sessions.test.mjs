import assert from "node:assert/strict";
import { test } from "node:test";
import { deriveSessionId, RELAY_TOOLS } from "../worker.mjs";
import { endpoint, post, Tab, setup, forward, reply } from "./test-support.mjs";

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

test("ready v2 stores bounded session metadata and derives a stable session id", () => {
  const tab = new Tab(0);
  const { room } = setup([tab]);
  const manyDocs = Array.from({ length: 50 }, (_, i) => ({
    tab: i,
    name: `file-${i}-${"x".repeat(150)}.fig`,
    key: "0123456789abcdef0123456789abcdef_extra",
    pages: i + 1,
  }));
  room.webSocketMessage(tab, readyV2("dev_office", "Office · Chrome", "inst_1", {
    focused: true,
    docs: manyDocs,
  }));
  const att = tab.deserializeAttachment();
  assert.equal(att.ready, true);
  assert.equal(att.sessionId, deriveSessionId("dev_office", "inst_1"));
  assert.match(att.sessionId, /^s_[0-9a-f]{8}$/);
  assert.equal(att.deviceId, "dev_office");
  assert.equal(att.deviceLabel, "Office · Chrome");
  assert.equal(att.focused, true);
  assert.equal(att.docs.length, 32);
  assert.equal(att.docs[0].name.length, 96);
  assert.equal(att.docs[0].key.length, 32);
  assert.ok(JSON.stringify(att).length < 16_384, "attachment must stay well under the 16 KiB DO limit");
});

test("tools/list works with no tab connected and augments tab tools when connected", async () => {
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
  const { frame, response } = await forward(room, tab, { jsonrpc: "2.0", id: 2, method: "tools/list" });
  reply(room, tab, frame, {
    body: {
      jsonrpc: "2.0",
      id: 2,
      result: {
        tools: [{
          name: "get_metadata",
          inputSchema: { type: "object", properties: { tab: { type: "integer" } } },
        }],
      },
    },
  });
  const listed = await (await response).json();
  const names = listed.result.tools.map((t) => t.name);
  assert.deepEqual(names, ["get_metadata", "list_sessions", "select_session"]);
  assert.equal(listed.result.tools[0].inputSchema.properties.session.type, "string");
});

test("single device with multiple tabs routes to focused or newest tab and tags _meta", async () => {
  const tabA = new Tab(1);
  const tabB = new Tab(2);
  const { room } = setup([tabA, tabB]);
  room.webSocketMessage(tabA, readyV2("dev_same", "Office MBP", "tab_a", { focused: true }));
  room.webSocketMessage(tabB, readyV2("dev_same", "Office MBP", "tab_b", { focused: false }));

  // Even though tabB sent ready later, tabA is focused on the same device so it wins.
  const sentA = new Promise((resolve) => { tabA.notify = resolve; });
  const resPromise = room.fetch(post(callTool("get_metadata", { node: "0:1" }, 10)));
  const frameA = await sentA;
  reply(room, tabA, frameA, {
    body: { jsonrpc: "2.0", id: 10, result: { content: [{ type: "text", text: "from A" }] } },
  });
  const bodyA = await (await resPromise).json();
  assert.equal(bodyA.result._meta["figstudio/session"], deriveSessionId("dev_same", "tab_a"));
  assert.equal(bodyA.result._meta["figstudio/device"], "Office MBP");
});

test("multiple devices without a session argument return -32002 with candidate sessions", async () => {
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

test("select_session pins the target across ready updates from other devices and resets on eviction", async () => {
  const office = new Tab(1);
  const home = new Tab(2);
  const { room } = setup([office, home]);
  room.webSocketMessage(office, readyV2("dev_office", "Office Mac", "inst_o"));
  room.webSocketMessage(home, readyV2("dev_home", "Home PC", "inst_h"));

  const homeSession = deriveSessionId("dev_home", "inst_h");
  const headers = { "Content-Type": "application/json", "Mcp-Session-Id": "client-1" };
  const postWithSession = (b) => new Request(endpoint, { method: "POST", body: JSON.stringify(b), headers });

  const selRes = await room.fetch(postWithSession(callTool("select_session", { session: homeSession }, 30)));
  assert.equal(selRes.status, 200);

  // Office sends ready again (e.g. a selection change or Wi-Fi reconnect blip):
  // sticky choice for client-1 must stay on home!
  room.webSocketMessage(office, readyV2("dev_office", "Office Mac", "inst_o", { focused: true }));

  const sentHome = new Promise((resolve) => { home.notify = resolve; });
  const callPromise = room.fetch(postWithSession(callTool("get_metadata", {}, 31)));
  const frame = await sentHome;
  reply(room, home, frame, {
    body: { jsonrpc: "2.0", id: 31, result: { content: [{ type: "text", text: "still home" }] } },
  });
  const callBody = await (await callPromise).json();
  assert.equal(callBody.result._meta["figstudio/session"], homeSession);
  assert.equal(office.sent.length, 0);

  // Simulated eviction: a new McpRoom instance has an empty sticky map and safely falls back to -32002.
  const { room: revived } = setup([office, home]);
  const afterEvict = await (await revived.fetch(postWithSession(callTool("get_metadata", {}, 32)))).json();
  assert.equal(afterEvict.error.code, -32002);
});
