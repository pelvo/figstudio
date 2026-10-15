import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// Run against a built Lane 2 app + this Worker, with cloud relay enabled.
const baseURL = process.env.RELAY_BASE_URL ?? "http://localhost:8787";
const fixture = fileURLToPath(new URL("../../tests/fixtures/structure.fig", import.meta.url));

test("room capability persists and recovers malformed storage", async ({ page, request }) => {
  test.setTimeout(180_000);
  const open = async () => {
    await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
    await expect.poll(() => page.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
      timeout: 60_000,
    }).toMatch(/^rm_[0-9a-f]{64}$/);
    const room = await page.evaluate(() => localStorage.getItem("figstudio.mcp.room"));
    // A real RPC, not just an allocated canvas, proves app and relay initialization.
    await expect.poll(async () => (await request.post(`${baseURL}/mcp/${room}`, {
      data: { jsonrpc: "2.0", id: 1, method: "ping" },
    })).status(), { timeout: 60_000 }).toBe(200);
    expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
    return room;
  };
  const room = await open();
  expect(await open()).toBe(room);
  await page.evaluate(() => localStorage.setItem("figstudio.mcp.room", "malformed"));
  expect(await open()).not.toBe(room);
});

test("browser executes MCP tools without uploading the local fig", async ({ page, request }) => {
  test.setTimeout(180_000);
  const bytes = await readFile(fixture);
  const violations = [];
  const bodies = [];
  const inspect = (body) => {
    if (!body || body.length === 0) return;
    bodies.push(body);
    if (body.includes(bytes.subarray(0, 64)) ||
        body.includes(Buffer.from(bytes.subarray(0, 63).toString("base64")))) {
      violations.push("fixture bytes left the browser");
    }
  };
  page.on("request", (req) => {
    const body = req.postDataBuffer();
    inspect(body);
    if (body && !new URL(req.url()).pathname.startsWith("/mcp/")) {
      violations.push("unexpected HTTP upload");
    }
  });
  page.on("websocket", (socket) => {
    socket.on("framesent", ({ payload }) => {
      if (typeof payload !== "string") {
        violations.push("binary WebSocket upload");
        inspect(payload);
        return;
      }
      inspect(Buffer.from(payload));
      if (payload === "ping") return;
      try {
        const frame = JSON.parse(payload);
        if (!["ready", "rpc_reply"].includes(frame.type)) violations.push("non-relay frame");
      } catch { violations.push("non-JSON WebSocket upload"); }
    });
  });

  await page.goto(baseURL);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toMatch(/^rm_[0-9a-f]{64}$/);
  const room = await page.evaluate(() => localStorage.getItem("figstudio.mcp.room"));
  const url = `${baseURL}/mcp/${room}`;
  let nextId = 1;
  const call = async (method, params) => {
    const id = nextId++;
    const body = { jsonrpc: "2.0", id, method, params };
    inspect(Buffer.from(JSON.stringify(body)));
    const response = await request.post(url, {
      data: body, headers: { Accept: "application/json, text/event-stream" },
    });
    expect(response.status()).toBe(200);
    const result = await response.json();
    expect(result.id).toBe(id);
    expect(result.error).toBeUndefined();
    return result.result;
  };
  const tool = async (name, args = {}) => {
    const result = await call("tools/call", { name, arguments: args });
    expect(result.isError).not.toBe(true);
    return result;
  };
  const structured = (result) => result.structuredContent ??
    JSON.parse(result.content.find((block) => block.type === "text").text);

  // egui renders to a canvas: use its Open shortcut rather than DOM-only selectors.
  await page.locator("#figstudio").click();
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press(process.platform === "darwin" ? "Meta+o" : "Control+o");
  await (await chooser).setFiles(fixture);
  await expect.poll(async () => (await request.post(url, {
    data: { jsonrpc: "2.0", id: 0, method: "ping" },
  })).status(), { timeout: 60_000 }).toBe(200);
  const initialized = await call("initialize", {
    protocolVersion: "2025-03-26", capabilities: {},
    clientInfo: { name: "relay-e2e", version: "1" },
  });
  expect(initialized.protocolVersion).toBe("2025-03-26");
  expect((await request.post(url, {
    data: { jsonrpc: "2.0", method: "notifications/initialized" },
  })).status()).toBe(202);

  let tabs;
  await expect.poll(async () => {
    tabs = structured(await tool("list_open_tabs")).tabs;
    return tabs.length;
  }, { timeout: 60_000 }).toBeGreaterThan(0);
  const tab = tabs.find((item) => item.active) ?? tabs[0];
  const args = { fileKey: tab.fileKey, nodeId: tab.page.id };
  const metadata = await tool("get_metadata", { fileKey: tab.fileKey });
  expect(metadata.content.some((block) => block.type === "text" && block.text.length > 0)).toBe(true);
  const screenshot = await tool("get_screenshot", { ...args, scale: 0.25 });
  const image = screenshot.content.find((block) => block.type === "image");
  expect(image.mimeType).toBe("image/png");
  expect(Buffer.from(image.data, "base64").subarray(0, 8))
    .toEqual(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]));
  await tool("reveal_node", args);
  await expect.poll(async () => structured(await tool("get_selection", {
    fileKey: tab.fileKey,
  })).node?.nodeId).toBe(args.nodeId);
  expect(bodies.length).toBeGreaterThan(0);
  expect(violations).toEqual([]);
});

test("a #room= link asks before joining; the joined tab becomes the executor", async ({
  page, browser, request,
}) => {
  test.setTimeout(480_000);
  const otherFixture = fileURLToPath(new URL("../../tests/fixtures/vector.fig", import.meta.url));
  const status = async (room) =>
    (await request.post(`${baseURL}/mcp/${room}`, {
      data: { jsonrpc: "2.0", id: 1, method: "ping" },
    })).status();
  // One initialized client per room, returning the structured tool results.
  const client = async (room) => {
    const post = (body) => request.post(`${baseURL}/mcp/${room}`, {
      data: body, headers: { Accept: "application/json, text-event-stream" },
    });
    expect((await post({
      jsonrpc: "2.0", id: 0, method: "initialize",
      params: { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "relay-e2e", version: "1" } },
    })).status()).toBe(200);
    let next = 0;
    return async (name, args = {}) => {
      const result = (await (await post({
        jsonrpc: "2.0", id: ++next, method: "tools/call", params: { name, arguments: args },
      })).json());
      expect(result.error).toBeUndefined();
      return result.result.structuredContent ?? JSON.parse(result.result.content[0].text);
    };
  };
  const openDoc = async (target, file) => {
    // egui needs the canvas focused right before the shortcut (the app's focus can drift).
    await target.locator("#figstudio").click();
    const chooser = target.waitForEvent("filechooser");
    await target.keyboard.press(process.platform === "darwin" ? "Meta+o" : "Control+o");
    await (await chooser).setFiles(file);
  };
  // Keyboard-only interaction with the confirmation dialog: Tab reaches Join, Enter picks
  // it. The waits let the dialog paint and take focus first (the app repaints at ≥4 Hz
  // while a proposal is pending).
  const approve = async (target) => {
    await target.locator("#figstudio").click();
    await target.waitForTimeout(600);
    await target.keyboard.press("Tab");
    await target.waitForTimeout(250);
    await target.keyboard.press("Enter");
  };

  // Browser A owns a room and keeps one document open in it.
  page.on("pageerror", (error) => console.log("[A] page error:", String(error).slice(0, 200)));
  await page.goto(baseURL);
  await page.locator("#figstudio").click();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toMatch(/^rm_[0-9a-f]{64}$/);
  const room = await page.evaluate(() => localStorage.getItem("figstudio.mcp.room"));
  await openDoc(page, fixture);
  const list = await client(room);
  await expect.poll(async () => (await list("list_open_tabs", {})).tabs.length, {
    timeout: 60_000,
  }).toBe(1);
  const firstKey = (await list("list_open_tabs", {})).tabs[0].fileKey;

  // A second browser (fresh storage, like another device) opens the room link.
  const other = await browser.newContext();
  const second = await other.newPage();
  const sockets = [];
  second.on("websocket", (socket) => sockets.push(socket.url()));
  second.on("pageerror", (error) => console.log("[second] page error:", String(error).slice(0, 200)));
  await second.goto(`${baseURL}/#room=${room}`);
  await second.locator("#figstudio").click();

  // Nothing joins without consent: the browser boots into its OWN generated room, never the
  // linked one …
  let own = null;
  await expect.poll(async () => {
    own = await second.evaluate(() => localStorage.getItem("figstudio.mcp.room"));
    return own !== null && own !== room && /^rm_[0-9a-f]{64}$/.test(own);
  }, { timeout: 60_000 }).toBeTruthy();
  // … the capability has already left the address bar, before any decision …
  await expect.poll(() => second.url()).not.toContain(room);
  // … and nothing connected to it.
  expect(sockets.filter((url) => url.includes(`/bridge/${room}`))).toEqual([]);

  // Declining keeps the old room, storage and connection: only the ask goes away.
  await second.keyboard.press("Escape");
  expect(await second.evaluate(() => localStorage.getItem("figstudio.mcp.room"))).toBe(own);
  expect(sockets.filter((url) => url.includes(`/bridge/${room}`))).toEqual([]);
  await expect.poll(async () => await status(own), { timeout: 60_000 }).toBe(200);

  // The link returns at runtime (hashchange) and is accepted: only now does anything join.
  await second.evaluate((token) => { location.hash = `#room=${token}`; }, room);
  await expect.poll(() => second.url()).not.toContain(room);
  await approve(second);
  await expect.poll(() => second.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toBe(room);
  expect(sockets.some((url) => url.includes(`/bridge/${room}`))).toBe(true);
  await expect.poll(async () => await status(room), { timeout: 60_000 }).toBe(200);

  // The second tab is the executor: the room now answers with its document, not A's.
  await openDoc(second, otherFixture);
  await expect.poll(async () => {
    const tabs = (await list("list_open_tabs", {})).tabs;
    return tabs.length === 1 && tabs[0].fileKey !== firstKey ? tabs[0].fileKey : false;
  }, { timeout: 60_000 }).toBeTruthy();

  // Another runtime link joins a third room; the newly joined room answers right away.
  const third = `rm_${"2".repeat(64)}`;
  await second.evaluate((token) => { location.hash = `#room=${token}`; }, third);
  await expect.poll(() => second.url()).not.toContain(third);
  await approve(second);
  await expect.poll(() => second.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toBe(third);
  await expect.poll(async () => await status(third), { timeout: 60_000 }).toBe(200);
  const listThird = await client(third);
  await expect.poll(async () => (await listThird("list_open_tabs", {})).tabs.length, {
    timeout: 60_000,
  }).toBe(1);

  // A changed `#mcp=` pairing wins over a `#room=` link, and the room key is still stripped.
  const stray = `rm_${"3".repeat(64)}`;
  await second.evaluate((token) => { location.hash = `#room=${token}&mcp=pairing-secret`; }, stray);
  await expect.poll(() => second.url()).not.toContain(stray);
  expect(second.url()).toContain("#mcp=");
  // No dialog was staged: approving nothing changes no room.
  await approve(second);
  expect(await second.evaluate(() => localStorage.getItem("figstudio.mcp.room"))).toBe(third);

  // An invalid room value is consumed but never proposed.
  await second.evaluate(() => { location.hash = "#room=rm_short"; });
  await expect.poll(() => second.url()).not.toContain("room=");
  await approve(second);
  expect(await second.evaluate(() => localStorage.getItem("figstudio.mcp.room"))).toBe(third);
  await other.close();
});
