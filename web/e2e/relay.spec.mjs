import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// Run against a built Lane 2 app + this Worker, with cloud relay enabled.
const baseURL = process.env.RELAY_BASE_URL ?? "http://localhost:8787";
const fixture = fileURLToPath(new URL("../../tests/fixtures/structure.fig", import.meta.url));

// Ctrl+O shows the File System Access picker where Chromium has it, and Playwright cannot
// drive that one (it cancels it). Without it the app shows the plain file input, which
// Playwright's file chooser fills; the relay does not care how the file was picked.
const plainPicker = () => { delete window.showOpenFilePicker; };
test.beforeEach(async ({ context }) => { await context.addInitScript(plainPicker); });

// The stored credential is the executor (`figstudio.mcp.executor.v2`, `ex_` + 64 hex);
// the caller room is derived from it — SHA-256 over "figstudio/room/v1", a zero byte,
// then the 32 raw executor bytes (same contract as web/executor-auth.mjs and
// apps/figstudio/src/mcp_pair.rs). Deriving it in the page also proves the derivation
// the app connects with.
const derivation = `(() => {
  const domain = new TextEncoder().encode("figstudio/room/v1");
  const derive = async (bytes) => {
    const pre = new Uint8Array(domain.length + 1 + bytes.length);
    pre.set(domain); pre[domain.length] = 0; pre.set(bytes, domain.length + 1);
    const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", pre));
    return "rm_" + [...digest].map((b) => b.toString(16).padStart(2, "0")).join("");
  };
  const read = async () => {
    const executor = localStorage.getItem("figstudio.mcp.executor.v2");
    if (!/^ex_[0-9a-f]{64}$/.test(executor)) return null;
    const bytes = Uint8Array.from(executor.slice(3).match(/../g), (h) => parseInt(h, 16));
    return { executor, room: await derive(bytes) };
  };
  return { derive, read };
})()`;

// `{ executor, room }` of the stored pair, derived in the page; `null` until stored.
const mcp = (page) => page.evaluate(`(${derivation}).read()`);

// A fresh executor pair minted in the page (for joining a room this browser never owned).
const mint = (page) => page.evaluate(`(async () => {
  const { derive } = ${derivation};
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  const hex = [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
  return { executor: "ex_" + hex, room: await derive(bytes) };
})()`);

// Opens `file` by dropping it on the canvas. The Open shortcut is unreliable here: the
// first launch's welcome sheet takes the keys, and the first frames may not take any.
const openFile = async (page, file) => {
  const bytes = (await readFile(file)).toString("base64");
  const name = file.split("/").pop();
  await page.evaluate(([bytes, name]) => {
    const raw = Uint8Array.from(atob(bytes), (c) => c.charCodeAt(0));
    const transfer = new DataTransfer();
    transfer.items.add(new File([raw], name));
    const canvas = document.getElementById("figstudio");
    for (const type of ["dragenter", "dragover", "drop"]) {
      canvas.dispatchEvent(new DragEvent(type, {
        bubbles: true, cancelable: true, dataTransfer: transfer,
      }));
    }
  }, [bytes, name]);
};

test("executor capability persists, migrates the legacy room and recovers malformed storage", async ({
  page, request,
}) => {
  test.setTimeout(180_000);
  const open = async () => {
    await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
    await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
    const stored = await mcp(page);
    expect(stored.executor).toMatch(/^ex_[0-9a-f]{64}$/);
    expect(stored.room).toMatch(/^rm_[0-9a-f]{64}$/);
    // A real RPC, not just an allocated canvas, proves app and relay initialization.
    await expect.poll(async () => (await request.post(`${baseURL}/mcp/${stored.room}`, {
      data: { jsonrpc: "2.0", id: 1, method: "ping" },
    })).status(), { timeout: 60_000 }).toBe(200);
    expect(await page.evaluate(() => crossOriginIsolated)).toBe(true);
    return stored;
  };
  const first = await open();
  expect((await open()).executor).toBe(first.executor);

  // A legacy caller-only room (`figstudio.mcp.room`) never becomes the credential: with
  // only it stored, a NEW executor pair is minted and saved, and the legacy key deleted.
  const legacyRoom = `rm_${"1".repeat(64)}`;
  await page.evaluate((room) => {
    localStorage.removeItem("figstudio.mcp.executor.v2");
    localStorage.setItem("figstudio.mcp.room", room);
  }, legacyRoom);
  const migrated = await open();
  expect(migrated.executor).not.toBe(first.executor);
  expect(migrated.room).not.toBe(legacyRoom);
  expect(await page.evaluate(() => localStorage.getItem("figstudio.mcp.room"))).toBeNull();

  // A malformed stored executor is visibly replaced, never silently kept.
  await page.evaluate(() => localStorage.setItem("figstudio.mcp.executor.v2", "malformed"));
  const recovered = await open();
  expect(recovered.executor).not.toBe(migrated.executor);
  expect(recovered.executor).toMatch(/^ex_[0-9a-f]{64}$/);
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
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  const { room } = await mcp(page);
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
  await openFile(page, fixture);
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
  // Revealing a page shows that page; it selects no layer (`Tab::show_node`).
  await expect.poll(async () => structured(await tool("get_selection", {
    fileKey: tab.fileKey,
  })).page?.id).toBe(args.nodeId);
  expect(bodies.length).toBeGreaterThan(0);
  expect(violations).toEqual([]);
});

test("a #room=&exec= link asks before joining; the joined tab becomes the executor", async ({
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
      data: body, headers: { Accept: "application/json, text/event-stream" },
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
  const openDoc = openFile;
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
  const pairLink = ({ room, executor }) => `#room=${room}&exec=${executor}`;
  const stored = (target) => target.evaluate(() =>
    localStorage.getItem("figstudio.mcp.executor.v2"));

  // Browser A owns a room and keeps one document open in it.
  page.on("pageerror", (error) => console.log("[A] page error:", String(error).slice(0, 200)));
  await page.goto(baseURL);
  await page.locator("#figstudio").click();
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  const own = await mcp(page);
  await openDoc(page, fixture);
  await expect.poll(() => status(own.room), { timeout: 60_000 }).toBe(200);
  const list = await client(own.room);
  await expect.poll(async () => (await list("list_open_tabs", {})).tabs.length, {
    timeout: 60_000,
  }).toBe(1);
  const firstKey = (await list("list_open_tabs", {})).tabs[0].fileKey;

  // The browser's own complete link is a no-op: the pair is already in use (no proposal),
  // and both secret keys still leave the address bar immediately.
  await page.goto(`${baseURL}/${pairLink(own)}`);
  await expect.poll(() => page.url()).not.toContain("room=");
  await expect.poll(() => page.url()).not.toContain("exec=");
  expect(await stored(page)).toBe(own.executor);
  await expect.poll(async () => await status(own.room), { timeout: 60_000 }).toBe(200);

  // A second browser (fresh storage, like another device) opens the room link.
  const other = await browser.newContext();
  await other.addInitScript(plainPicker);
  const second = await other.newPage();
  const sockets = [];
  second.on("websocket", (socket) => sockets.push(socket.url()));
  second.on("pageerror", (error) => console.log("[second] page error:", String(error).slice(0, 200)));
  await second.goto(`${baseURL}/${pairLink(own)}`);
  await second.locator("#figstudio").click();

  // Nothing joins without consent: the browser boots into its OWN generated pair, never
  // the linked one …
  let secondOwn = null;
  await expect.poll(async () => {
    secondOwn = await mcp(second);
    return secondOwn !== null && secondOwn.executor !== own.executor;
  }, { timeout: 60_000 }).toBeTruthy();
  expect(secondOwn.room).not.toBe(own.room);
  // … the capability has already left the address bar, before any decision …
  await expect.poll(() => second.url()).not.toContain(own.executor);
  await expect.poll(() => second.url()).not.toContain(own.room);
  // … and nothing connected to it.
  expect(sockets.filter((url) => url.includes(`/bridge/${own.room}`))).toEqual([]);

  // Declining keeps the old pair, storage and connection: only the ask goes away.
  await second.keyboard.press("Escape");
  expect(await stored(second)).toBe(secondOwn.executor);
  expect(sockets.filter((url) => url.includes(`/bridge/${own.room}`))).toEqual([]);
  await expect.poll(async () => await status(secondOwn.room), { timeout: 60_000 }).toBe(200);

  // The link returns at runtime (hashchange) and is accepted: only now does anything join.
  await second.evaluate((link) => { location.hash = link; }, pairLink(own));
  await expect.poll(() => second.url()).not.toContain(own.executor);
  await approve(second);
  await expect.poll(() => stored(second), { timeout: 60_000 }).toBe(own.executor);
  // The join saves first; the reconnect happens on a later frame, so poll for the socket.
  await expect.poll(() => sockets.some((url) => url.includes(`/bridge/${own.room}`)))
    .toBe(true);
  await expect.poll(async () => await status(own.room), { timeout: 60_000 }).toBe(200);

  // The second tab is the executor: the room now answers with its document, not A's.
  await openDoc(second, otherFixture);
  await expect.poll(async () => {
    const tabs = (await list("list_open_tabs", {})).tabs;
    return tabs.length === 1 && tabs[0].fileKey !== firstKey ? tabs[0].fileKey : false;
  }, { timeout: 60_000 }).toBeTruthy();

  // Another runtime link joins a third room — a freshly minted pair, joined by its
  // complete link; the newly joined room answers right away.
  const third = await mint(second);
  await second.evaluate((link) => { location.hash = link; }, pairLink(third));
  await expect.poll(() => second.url()).not.toContain(third.executor);
  await approve(second);
  await expect.poll(() => stored(second), { timeout: 60_000 }).toBe(third.executor);
  await expect.poll(async () => await status(third.room), { timeout: 60_000 }).toBe(200);
  const listThird = await client(third.room);
  await expect.poll(async () => (await listThird("list_open_tabs", {})).tabs.length, {
    timeout: 60_000,
  }).toBe(1);

  // A caller-only possession — the room without its executor — is consumed by the
  // fragment parser but never proposes: there is nothing to join with.
  await second.evaluate((room) => { location.hash = `#room=${room}`; }, third.room);
  await expect.poll(() => second.url()).not.toContain("room=");
  await approve(second);
  expect(await stored(second)).toBe(third.executor);

  // A changed `#mcp=` pairing wins over a pair link, and both secret keys are stripped.
  const stray = await mint(second);
  await second.evaluate((link) => { location.hash = `${link}&mcp=pairing-secret`; }, pairLink(stray));
  await expect.poll(() => second.url()).not.toContain("room=");
  await expect.poll(() => second.url()).not.toContain("exec=");
  expect(second.url()).toContain("#mcp=");
  // No dialog was staged: approving nothing changes no pair.
  await approve(second);
  expect(await stored(second)).toBe(third.executor);

  // Invalid values are consumed but never proposed.
  await second.evaluate(() => { location.hash = "#room=rm_short&exec=ex_short"; });
  await expect.poll(() => second.url()).not.toContain("room=");
  await approve(second);
  expect(await stored(second)).toBe(third.executor);
  await other.close();
});

test("the bridge upgrade needs the executor proof, not the caller room", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  const { room, executor } = await mcp(page);
  // In the page the Origin is right by construction, so this pins the one gate the
  // Origin is not: possession of the caller room alone (the MCP URL) must not open the
  // bridge, while the two-subprotocol proof must.
  const attempt = (protocols) => page.evaluate(async ({ room, protocols }) => {
    const url = `${location.origin.replace("http", "ws")}/bridge/${room}`;
    return await new Promise((resolve) => {
      let settled = false;
      const done = (outcome) => { if (!settled) { settled = true; resolve(outcome); } };
      const ws = protocols ? new WebSocket(url, protocols) : new WebSocket(url);
      ws.onopen = () => { done("open"); ws.close(); };
      ws.onerror = () => done("error");
      ws.onclose = () => done("error");
      setTimeout(() => done("timeout"), 10_000);
    });
  }, { room, protocols: protocols ?? null });
  expect(await attempt(null)).toBe("error");
  expect(await attempt(["figstudio-exec-v1"])).toBe("error");
  expect(await attempt(["figstudio-exec-v1", "0".repeat(64)])).toBe("error");
  // The selected subprotocol is the constant marker, never the executor hex.
  const opened = await page.evaluate(async ({ room, protocols }) => {
    const url = `${location.origin.replace("http", "ws")}/bridge/${room}`;
    return await new Promise((resolve, reject) => {
      const ws = new WebSocket(url, protocols);
      ws.onopen = () => { resolve(ws.protocol); ws.close(); };
      ws.onerror = () => reject(new Error("proof refused"));
      setTimeout(() => reject(new Error("timeout")), 10_000);
    });
  }, { room, protocols: ["figstudio-exec-v1", executor.slice(3)] });
  expect(opened).toBe("figstudio-exec-v1");
});

test("a relay socket whose keepalives go unanswered is closed by the page", async ({ page }) => {
  test.setTimeout(180_000);
  // Blackhole the keepalive answer at the transport: the page's `ping` frames are
  // forwarded, the runtime's auto-`pong` is swallowed, so the socket stays open at the
  // API level while no end-to-end proof can ever arrive — the silently-dropped NAT
  // path `mcp_heartbeat.rs` exists for. This observes the real connection the app
  // holds, not a helper's answer.
  // Count only animation frames that really draw: eframe requests idle frames too.
  // A draw later in the close's own frame still contains the old status label.
  await page.addInitScript(() => {
    let frame = 0;
    const request = window.requestAnimationFrame;
    window.requestAnimationFrame = (callback) => request.call(window, (time) => {
      frame++;
      callback(time);
    });
    window.relayPaint = { paintedFrame: 0, atClose: null, renderers: 0 };
    const close = WebSocket.prototype.close;
    WebSocket.prototype.close = function (...args) {
      if (this.url.includes("/bridge/")) {
        window.relayPaint.atClose = frame;
      }
      return close.apply(this, args);
    };
    // Headless Chromium may fall back to WebGL2; count either renderer's output. A
    // missing interface is skipped so it cannot abort the script before the close hook.
    const gpu = globalThis.GPUQueue?.prototype;
    const gl = globalThis.WebGL2RenderingContext?.prototype;
    for (const [prototype, method] of [
      [gpu, "submit"],
      [gl, "drawElements"],
      [gl, "drawArrays"],
      [gl, "drawElementsInstanced"],
      [gl, "drawArraysInstanced"],
    ]) {
      if (!prototype?.[method]) continue;
      window.relayPaint.renderers++;
      const original = prototype[method];
      prototype[method] = function (...args) {
        window.relayPaint.paintedFrame = frame;
        return original.apply(this, args);
      };
    }
  });
  const sockets = [];
  let closedByPage = false;
  let pings = 0;
  let pongs = 0;
  await page.routeWebSocket(/\/bridge\//, (ws) => {
    sockets.push(ws);
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      if (message === "ping") pings++;
      server.send(message);
    });
    server.onMessage((message) => {
      if (message === "pong") pongs++;
      else ws.send(message);
    });
    ws.onClose(() => {
      closedByPage = true;
      server.close();
    });
  });
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  // The interception is live: the page's first keepalive went out through it, and the
  // runtime answered a pong the page never received.
  await expect.poll(() => pings > 0 && pongs > 0, { timeout: 30_000 }).toBe(true);
  // With no pong the 10 s deadline (mcp_heartbeat.rs PONG_DEADLINE) must close the
  // socket from our side — the real connection, not a helper's answer.
  await expect
    .poll(() => closedByPage, { timeout: 60_000 })
    .toBe(true);
  // Status is drawn before sync detects the loss. The real Wasm UI must paint
  // another frame to remove Connected and consume its notice, without user input.
  expect(await page.evaluate(() => window.relayPaint.renderers), "no draw hook installed")
    .toBeGreaterThan(0);
  await expect.poll(() => page.evaluate(() => {
    const { paintedFrame, atClose } = window.relayPaint;
    return atClose !== null && atClose > 0 && paintedFrame > atClose;
  }), { timeout: 5_000 }).toBe(true);
  await test.info().attach("relay-disconnected", {
    body: await page.screenshot({ path: test.info().outputPath("relay-disconnected.png") }),
    contentType: "image/png",
  });
  // Retried with growing delays (mcp_backoff.rs): the next socket opens about 1 s after
  // the close, and the black-holed path costs one attempt per delay, not a storm. The
  // replacement is black-holed too; its own deadline is 10 s away, so it stays alone.
  await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
  await page.waitForTimeout(5_000);
  expect(sockets).toHaveLength(2);
});

test("a refused ping drops the relay and retries it with growing delays", async ({ page }) => {
  test.setTimeout(150_000);
  await page.addInitScript(() => {
    const send = WebSocket.prototype.send;
    window.relaySend = { opened: [], pongs: 0, pings: 0, refused: 0 };
    const NativeWebSocket = WebSocket;
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        if (this.url.includes("/bridge/")) {
          window.relaySend.opened.push(performance.now());
          this.addEventListener("message", (event) => {
            if (event.data === "pong") window.relaySend.pongs++;
          });
        }
      }
    };
    WebSocket.prototype.send = function (message) {
      if (this.url.includes("/bridge/") && message === "ping") {
        // Let the first real ping/pong prove the original connection, then refuse
        // sends on it and every replacement through the actual Wasm error path.
        if (++window.relaySend.pings > 1) {
          window.relaySend.refused++;
          throw new DOMException("send refused", "InvalidStateError");
        }
      }
      return send.call(this, message);
    };
  });
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect.poll(() => page.evaluate(() => window.relaySend.pongs), { timeout: 60_000 }).toBe(1);
  // The proven socket's next ping (30 s later) is refused; then each replacement's first.
  await expect.poll(() => page.evaluate(() => window.relaySend.opened.length), {
    timeout: 90_000,
  }).toBeGreaterThanOrEqual(4);
  const { opened, pongs } = await page.evaluate(() => window.relaySend);
  const gaps = opened.slice(1).map((time, i) => time - opened[i]);
  // After the first replacement the delays double: 2 s, then 4 s (mcp_backoff.rs).
  expect(gaps[1]).toBeGreaterThan(1_500);
  expect(gaps[2]).toBeGreaterThan(3_500);
  expect(gaps[2]).toBeGreaterThan(gaps[1]);
  expect(pongs).toBe(1);
});

test("a hidden tab answers relay calls while its frames are throttled", async ({ page, request }) => {
  test.setTimeout(180_000);
  // A background tab: `document.hidden` turns true, eframe then runs only `App::logic`
  // on a timer, and that timer is held to once a minute as Chrome's intensive
  // throttling does after a few minutes hidden. Calls must still be answered well inside
  // the relay's 30 s deadline (mcp_web_calls.rs, web_wake.rs).
  await page.addInitScript(() => {
    window.relayHidden = false;
    const hidden = Object.getOwnPropertyDescriptor(Document.prototype, "hidden");
    Object.defineProperty(Document.prototype, "hidden", {
      configurable: true,
      get() { return window.relayHidden || hidden.get.call(this); },
    });
    Object.defineProperty(Document.prototype, "visibilityState", {
      configurable: true,
      get() { return window.relayHidden ? "hidden" : "visible"; },
    });
    const later = window.setTimeout;
    // eframe's hidden-tab frame timer asks for 10 ms; nothing else in the page does.
    window.setTimeout = (callback, delay, ...rest) =>
      later.call(window, callback, window.relayHidden && delay === 10 ? 60_000 : delay, ...rest);
  });
  await page.goto(baseURL);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  const { room } = await mcp(page);
  const url = `${baseURL}/mcp/${room}`;
  await openFile(page, fixture);
  const tool = (id, name, args = {}) => request.post(url, {
    data: { jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments: args } },
    timeout: 60_000,
  });
  let fileKey;
  let nodeId;
  await expect.poll(async () => {
    const response = await tool(1, "list_open_tabs");
    if (response.status() !== 200) return 0;
    const result = (await response.json()).result;
    const tabs = (result.structuredContent ??
      JSON.parse(result.content.find((block) => block.type === "text").text)).tabs;
    fileKey = tabs[0]?.fileKey;
    nodeId = tabs[0]?.page.id;
    return tabs.length;
  }, { timeout: 60_000 }).toBeGreaterThan(0);

  await page.evaluate(() => {
    window.relayHidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Let the last visible-timer frame pass, so the next one is a minute away.
  await page.waitForTimeout(2_000);
  for (const [id, name, args] of [
    [2, "get_metadata", { fileKey }],
    [3, "get_screenshot", { fileKey, nodeId, scale: 0.25 }],
  ]) {
    const started = Date.now();
    const response = await tool(id, name, args);
    expect(response.status(), `${name} while hidden`).toBe(200);
    const body = await response.json();
    expect(body.error).toBeUndefined();
    expect(body.result.isError, JSON.stringify(body.result.content)).not.toBe(true);
    expect(Date.now() - started, `${name} answered without a frame`).toBeLessThan(10_000);
  }
});

test("a hidden tab reconnects a dropped relay without waiting for a frame", async ({
  page, request,
}) => {
  test.setTimeout(180_000);
  // The same background tab as above: its frames are a minute apart. A dropped relay
  // socket (a Worker deploy, a network change) must still be noticed, retried and greeted
  // with `ready` between frames (mcp_web_pump.rs), or every call meanwhile gets 503.
  await page.addInitScript(() => {
    window.relayHidden = false;
    window.relaySockets = [];
    const hidden = Object.getOwnPropertyDescriptor(Document.prototype, "hidden");
    Object.defineProperty(Document.prototype, "hidden", {
      configurable: true,
      get() { return window.relayHidden || hidden.get.call(this); },
    });
    Object.defineProperty(Document.prototype, "visibilityState", {
      configurable: true,
      get() { return window.relayHidden ? "hidden" : "visible"; },
    });
    const later = window.setTimeout;
    window.setTimeout = (callback, delay, ...rest) =>
      later.call(window, callback, window.relayHidden && delay === 10 ? 60_000 : delay, ...rest);
    const NativeWebSocket = WebSocket;
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        if (String(args[0]).includes("/bridge/")) window.relaySockets.push(this);
      }
    };
  });
  await page.goto(baseURL);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  const { room } = await mcp(page);
  const url = `${baseURL}/mcp/${room}`;
  await openFile(page, fixture);
  let nextId = 1;
  const tabs = async () => {
    const response = await request.post(url, {
      data: {
        jsonrpc: "2.0", id: nextId++, method: "tools/call",
        params: { name: "list_open_tabs", arguments: {} },
      },
    });
    if (response.status() !== 200) return 0;
    const result = (await response.json()).result;
    if (result.isError) return 0;
    return (result.structuredContent ??
      JSON.parse(result.content.find((block) => block.type === "text").text)).tabs.length;
  };
  await expect.poll(tabs, { timeout: 60_000 }).toBeGreaterThan(0);

  await page.evaluate(() => {
    window.relayHidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Let the last visible-timer frame pass, so the next one is a minute away.
  await page.waitForTimeout(2_000);
  expect(await page.evaluate(() => window.relaySockets.length)).toBe(1);
  const dropped = Date.now();
  await page.evaluate(() => window.relaySockets[0].close());
  await expect.poll(tabs, { timeout: 20_000, intervals: [250] }).toBeGreaterThan(0);
  expect(Date.now() - dropped, "reconnected without a frame").toBeLessThan(15_000);
  expect(await page.evaluate(() => window.relaySockets.length)).toBe(2);
});

test("a hidden tab retries a relay whose keepalive went unanswered", async ({ page }) => {
  test.setTimeout(180_000);
  // The silent loss in a background tab: a socket whose `pong` never comes back fires no
  // `close`, so only the keepalive deadline notices it, and the retry after that must be
  // timed between frames too (`relay_lost`), not by eframe's minute-long hidden timer.
  await page.addInitScript(() => {
    window.relayHidden = false;
    const hidden = Object.getOwnPropertyDescriptor(Document.prototype, "hidden");
    Object.defineProperty(Document.prototype, "hidden", {
      configurable: true,
      get() { return window.relayHidden || hidden.get.call(this); },
    });
    Object.defineProperty(Document.prototype, "visibilityState", {
      configurable: true,
      get() { return window.relayHidden ? "hidden" : "visible"; },
    });
    const later = window.setTimeout;
    window.setTimeout = (callback, delay, ...rest) =>
      later.call(window, callback, window.relayHidden && delay === 10 ? 60_000 : delay, ...rest);
  });
  // The first socket works; every later one has its `pong` swallowed.
  const sockets = [];
  let pongs = 0;
  await page.routeWebSocket(/\/bridge\//, (ws) => {
    const index = sockets.push(ws) - 1;
    const server = ws.connectToServer();
    ws.onMessage((message) => server.send(message));
    server.onMessage((message) => {
      if (message !== "pong") ws.send(message);
      else if (index === 0 && ++pongs) ws.send(message);
    });
    ws.onClose(() => server.close());
  });
  await page.goto(baseURL);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => mcp(page), { timeout: 60_000 }).toBeTruthy();
  await expect.poll(() => pongs, { timeout: 30_000 }).toBeGreaterThan(0);

  await page.evaluate(() => {
    window.relayHidden = true;
    document.dispatchEvent(new Event("visibilitychange"));
  });
  // Let the last visible-timer frame pass, so the next one is a minute away.
  await page.waitForTimeout(2_000);
  const dropped = Date.now();
  sockets[0].close();
  // The replacement opens about 1 s later; its first ping goes unanswered, its 10 s
  // deadline expires, and the next attempt follows 2 s after that.
  await expect.poll(() => sockets.length, { timeout: 10_000 }).toBe(2);
  await expect.poll(() => sockets.length, { timeout: 40_000 }).toBe(3);
  expect(Date.now() - dropped, "retried without a frame").toBeLessThan(25_000);
});
