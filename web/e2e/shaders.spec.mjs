import { test, expect } from "@playwright/test";
import { readFile, mkdtemp } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";

// A shader fill drawn in the browser: its code (a public fixture's source, zipped here) is
// dropped on the page and run on WebGPU in the sandboxed frame. Needs a browser with WebGPU.
const baseURL = process.env.FIGSTUDIO_BASE_URL ?? "http://127.0.0.1:8765";

// Full Chromium (new headless) for a working GPU canvas; CHROMIUM_PATH picks a local build.
test.use({
  channel: "chromium",
  launchOptions: { args: ["--enable-unsafe-webgpu"], executablePath: process.env.CHROMIUM_PATH },
});

async function drop(page, name, bytes) {
  const transfer = await page.evaluateHandle(({ name, encoded }) => {
    const data = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
    const transfer = new DataTransfer();
    transfer.items.add(new File([data], name, { type: "application/octet-stream" }));
    return transfer;
  }, { name, encoded: bytes.toString("base64") });
  await page.locator("#figstudio").dispatchEvent("drop", { dataTransfer: transfer });
}

test("a dropped shader zip draws its fill on the page's WebGPU, sandboxed", async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect(page.locator("#status")).toHaveText("", { timeout: 60_000 });
  await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
  await page.locator("#figstudio").focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);
  test.skip(!(await page.evaluate(() => !!navigator.gpu)), "no WebGPU in this browser");

  const status = () => page.evaluate(() => JSON.parse(window.figstudioStatus ?? "{}"));
  const fig = await readFile(new URL("../../tests/fixtures/shaders.fig", import.meta.url));
  // A drop before the app listens is lost: drop again until the file is open.
  for (let i = 0; i < 12 && !((await status()).tabs > 0); i++) {
    await drop(page, "shaders.fig", fig);
    await expect.poll(async () => (await status()).tabs ?? 0, { timeout: 5_000 }).toBeGreaterThan(0).catch(() => {});
  }
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000, message: JSON.stringify(await status()) }).toBe(12);

  const dir = await mkdtemp(join(tmpdir(), "figstudio-shader-"));
  const zip = join(dir, "Concentric-patterns-source.zip");
  const source = new URL("../../tests/fixtures/shader-sources/concentric", import.meta.url).pathname;
  execFileSync("zip", ["-q", "-j", zip, join(source, "main.ts"), join(source, "features.json")]);
  const runs = () => page.evaluate(() => globalThis.__figstudioShaders?.runs ?? { ok: 0, failed: 0 });
  await drop(page, "Concentric-patterns-source.zip", await readFile(zip));
  // Its code is taken, and the shader ran in the sandbox.
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000 }).toBe(11);
  await expect.poll(async () => (await runs()).ok, { timeout: 60_000 }).toBeGreaterThan(0);
  expect((await runs()).failed, (await runs()).lastError ?? "").toBe(0);
  // The shader ran in an opaque-origin frame.
  expect(await page.locator("iframe[sandbox='allow-scripts']").count()).toBe(1);
  expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
  expect(errors).toEqual([]);
  await page.waitForTimeout(1_000);
  await page.screenshot({ path: testInfo.outputPath("shaders.jpg"), type: "jpeg", quality: 85 });
});

// Opens shaders.fig and drops the Concentric shader's real source with `body` put at the start
// of its render function; returns the status reader.
async function dropStuck(page, body) {
  await page.goto(`${baseURL}/?renderer=cpu&threads=2`);
  await expect(page.locator("#status")).toHaveText("", { timeout: 60_000 });
  await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
  await page.locator("#figstudio").focus();
  await page.keyboard.press("Escape");
  await page.waitForTimeout(600);
  test.skip(!(await page.evaluate(() => !!navigator.gpu)), "no WebGPU in this browser");

  const status = () => page.evaluate(() => JSON.parse(window.figstudioStatus ?? "{}"));
  const fig = await readFile(new URL("../../tests/fixtures/shaders.fig", import.meta.url));
  for (let i = 0; i < 12 && !((await status()).tabs > 0); i++) {
    await drop(page, "shaders.fig", fig);
    await expect.poll(async () => (await status()).tabs ?? 0, { timeout: 5_000 }).toBeGreaterThan(0).catch(() => {});
  }
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000 }).toBe(12);

  const dir = await mkdtemp(join(tmpdir(), "figstudio-stuck-"));
  const source = new URL("../../tests/fixtures/shader-sources/concentric", import.meta.url).pathname;
  const head = "export function render(device: any, frame: any) {";
  const main = (await readFile(join(source, "main.ts"), "utf8")).replace(head, `${head} ${body}`);
  const { writeFile } = await import("node:fs/promises");
  await writeFile(join(dir, "main.ts"), main);
  const zip = join(dir, "Concentric-patterns-source.zip");
  execFileSync("zip", ["-q", "-j", zip, join(dir, "main.ts"), join(source, "features.json")]);
  await drop(page, "Concentric-patterns-source.zip", await readFile(zip));
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000 }).toBe(11);
  return status;
}

/// While the shader is stuck the page answers; the run then fails with `reason`.
async function expectStopped(page, status, reason, errors) {
  const runs = () => page.evaluate(() => globalThis.__figstudioShaders?.runs ?? { failed: 0 });
  let worst = 0;
  for (let i = 0; i < 60 && (await runs()).failed === 0; i++) {
    const asked = Date.now();
    await status();
    worst = Math.max(worst, Date.now() - asked);
    await page.waitForTimeout(250);
  }
  expect((await runs()).failed).toBeGreaterThan(0);
  expect((await runs()).lastError).toContain(reason);
  expect(worst).toBeLessThan(1_000);
  expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
  expect(errors).toEqual([]);
}

test("a shader that never returns is stopped and the page keeps answering", async ({ page }) => {
  test.setTimeout(180_000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  // Its loops count steps, so it stops itself (where its frame shares the page's thread too).
  const status = await dropStuck(page, "for (;;) {}");
  await expectStopped(page, status, "step limit", errors);
});

test("a built-in that never returns is stopped with its worker", async ({ page }) => {
  test.setTimeout(180_000);
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  // No JavaScript runs while it walks a trillion empty indexes, so no step is counted: the
  // worker (its own thread) is ended with its frame after the timeout.
  const status = await dropStuck(page, "Array.prototype.forEach.call({ length: 1e12 }, () => {});");
  await expectStopped(page, status, "took too long", errors);
});
