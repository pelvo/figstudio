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

test("a shader that never returns is stopped and the page keeps answering", async ({ page }) => {
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
  for (let i = 0; i < 12 && !((await status()).tabs > 0); i++) {
    await drop(page, "shaders.fig", fig);
    await expect.poll(async () => (await status()).tabs ?? 0, { timeout: 5_000 }).toBeGreaterThan(0).catch(() => {});
  }
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000 }).toBe(12);

  // The real source (its parameters match the shader) with an endless loop in render.
  const dir = await mkdtemp(join(tmpdir(), "figstudio-endless-"));
  const source = new URL("../../tests/fixtures/shader-sources/concentric", import.meta.url).pathname;
  const head = "export function render(device: any, frame: any) {";
  const main = (await readFile(join(source, "main.ts"), "utf8")).replace(head, `${head} for (;;) {}`);
  const { writeFile } = await import("node:fs/promises");
  await writeFile(join(dir, "main.ts"), main);
  const zip = join(dir, "Concentric-patterns-source.zip");
  execFileSync("zip", ["-q", "-j", zip, join(dir, "main.ts"), join(source, "features.json")]);
  await drop(page, "Concentric-patterns-source.zip", await readFile(zip));
  await expect.poll(async () => (await status()).shaders_missing, { timeout: 30_000 }).toBe(11);

  // Its loops count steps and it stops (where its frame shares the page's thread too).
  const runs = () => page.evaluate(() => globalThis.__figstudioShaders?.runs ?? { failed: 0 });
  await expect.poll(async () => (await runs()).failed, { timeout: 30_000 }).toBeGreaterThan(0);
  expect((await runs()).lastError).toContain("step limit");
  const asked = Date.now();
  await status();
  expect(Date.now() - asked).toBeLessThan(1_000);
  expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
  expect(errors).toEqual([]);
});
