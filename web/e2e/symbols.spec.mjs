import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

// Uses the existing browser harness and a public fixture, no relay or private files.
const baseURL = process.env.FIGSTUDIO_BASE_URL ?? "http://127.0.0.1:8765";

test("missing UI symbols fetch outline fonts lazily without stopping the app", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const fonts = new Map();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("response", response => {
    const name = new URL(response.url()).pathname.split("/").at(-1);
    if (["symbols.ttf", "symbols2.ttf", "math.ttf", "emoji.ttf"].includes(name)) {
      void response.finished().then(() => fonts.set(name, response.status()));
    }
  });
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect(page.locator("#status")).toHaveText("", { timeout: 60_000 });
  await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
  await page.locator("#figstudio").focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(fonts.size).toBe(0);
  const buffer = await readFile(new URL("../../tests/fixtures/corners.fig", import.meta.url));
  const open = async name => {
    const transfer = await page.evaluateHandle(({ name, encoded }) => {
      const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
      const transfer = new DataTransfer();
      transfer.items.add(new File([bytes], name, { type: "application/octet-stream" }));
      return transfer;
    }, { name, encoded: buffer.toString("base64") });
    await page.locator("#figstudio").dispatchEvent("drop", { dataTransfer: transfer });
  };
  await open("📄 └ symbols.fig");
  await expect.poll(() => fonts.size, { timeout: 30_000 }).toBe(4);
  expect([...fonts.values()]).toEqual([200, 200, 200, 200]);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() =>
    requestAnimationFrame(() => requestAnimationFrame(resolve)))));
  expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
  expect(errors).toEqual([]);
  await page.screenshot({ path: testInfo.outputPath("symbols-ui.jpg"), type: "jpeg", quality: 85 });
});

test("Recents labels fetch missing symbol faces without an open document", async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const expected = ["symbols.ttf", "symbols2.ttf", "math.ttf", "emoji.ttf"].sort();
  const requests = [];
  const statuses = [];
  page.on("request", request => {
    const name = new URL(request.url()).pathname.split("/").at(-1);
    if (expected.includes(name)) requests.push(name);
  });
  page.on("response", response => {
    const name = new URL(response.url()).pathname.split("/").at(-1);
    if (expected.includes(name)) void response.finished().then(() => statuses.push(response.status()));
  });
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(requests).toEqual([]);
  await page.evaluate(() => new Promise((resolve, reject) => {
    const open = indexedDB.open("figstudio-recents", 1);
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result;
      const tx = db.transaction("list", "readwrite");
      tx.objectStore("list").put(JSON.stringify([{ path: "└ recent.fig", opened: 1 }]), "entries");
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => { db.close(); reject(tx.error); };
    };
  }));
  // Reload only the saved list. No document is opened by this test.
  await page.reload();
  await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
  await page.locator("#figstudio").focus();
  await page.keyboard.press("Escape");
  await expect.poll(() => statuses.length, { timeout: 30_000 }).toBe(4);
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(requests.sort()).toEqual(expected);
  expect(statuses).toEqual([200, 200, 200, 200]);
  expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
  console.log(`Recents without a document: ${requests.length} symbol font requests`);
  await page.screenshot({ path: testInfo.outputPath("recents-symbol-title.jpg"), type: "jpeg", quality: 85 });
});
