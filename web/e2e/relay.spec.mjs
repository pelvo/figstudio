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

test("a #room= link joins that room and the fragment is consumed", async ({ page, browser, request }) => {
  test.setTimeout(180_000);
  // A first browser owns a room.
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect.poll(() => page.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toMatch(/^rm_[0-9a-f]{64}$/);
  const room = await page.evaluate(() => localStorage.getItem("figstudio.mcp.room"));

  // A second browser joins by opening the room link (fresh storage, like another device).
  const other = await browser.newContext();
  const second = await other.newPage();
  await second.goto(`${baseURL}/?renderer=cpu&threads=2#room=${room}`);
  await expect.poll(() => second.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toBe(room);
  // The token does not stay in the address bar, the history or a copied URL.
  await expect.poll(() => second.url()).not.toContain("#");
  // The join really reconnected the relay.
  await expect.poll(async () => (await request.post(`${baseURL}/mcp/${room}`, {
    data: { jsonrpc: "2.0", id: 1, method: "ping" },
  })).status(), { timeout: 60_000 }).toBe(200);

  // A fragment gained after startup (hashchange) joins the same way.
  const linked = `rm_${"1".repeat(64)}`;
  await second.evaluate((token) => { location.hash = `#room=${token}`; }, linked);
  await expect.poll(() => second.evaluate(() => localStorage.getItem("figstudio.mcp.room")), {
    timeout: 60_000,
  }).toBe(linked);
  await expect.poll(() => second.url()).not.toContain("#");
  await other.close();
});
