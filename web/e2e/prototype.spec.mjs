import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

// The prototype built from the public structure.fig fixture; write it with
//   FIGSTUDIO_PROTOTYPE_FIXTURE=/tmp/prototype.fig cargo test -p figstudio-render --test prototype -- --ignored write_fixture
const fixture = process.env.FIGSTUDIO_PROTOTYPE_FIXTURE;
const baseURL = process.env.FIGSTUDIO_BASE_URL ?? "http://127.0.0.1:8765";
const status = page => page.evaluate(() => JSON.parse(window.figstudioStatus));

/// Compares two PNG screenshots in the page: how many pixels differ, and the box of the
/// pixels that turned bluer (Preview's link flash).
const compare = (page, a, b) => page.evaluate(async ([a, b]) => {
  const pixels = async encoded => {
    const blob = await (await fetch(`data:image/png;base64,${encoded}`)).blob();
    const bitmap = await createImageBitmap(blob);
    const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
    ctx.drawImage(bitmap, 0, 0);
    return { width: bitmap.width, data: ctx.getImageData(0, 0, bitmap.width, bitmap.height).data };
  };
  const [pa, pb] = [await pixels(a), await pixels(b)];
  let changed = 0, box = null;
  for (let i = 0; i < pa.data.length; i += 4) {
    const d = Math.abs(pa.data[i] - pb.data[i]) + Math.abs(pa.data[i + 1] - pb.data[i + 1]) + Math.abs(pa.data[i + 2] - pb.data[i + 2]);
    if (d > 30) changed++;
    if ((pb.data[i + 2] - pb.data[i]) - (pa.data[i + 2] - pa.data[i]) > 20) {
      const x = (i / 4) % pa.width, y = Math.floor(i / 4 / pa.width);
      box = box ? { x0: Math.min(box.x0, x), y0: Math.min(box.y0, y), x1: Math.max(box.x1, x), y1: Math.max(box.y1, y) } : { x0: x, y0: y, x1: x, y1: y };
    }
  }
  return { changed, box };
}, [a.toString("base64"), b.toString("base64")]);

test("Preview: Shift+Space, a missed click flashes the link, the link navigates, Back returns, Esc exits", async ({ page }, testInfo) => {
  test.skip(!fixture, "set FIGSTUDIO_PROTOTYPE_FIXTURE to the generated prototype file");
  test.setTimeout(180_000);
  await page.goto(`${baseURL}/?threads=2`);
  await expect(page.locator("#status")).toHaveText("", { timeout: 90_000 });
  await page.locator("#figstudio").focus();
  await page.keyboard.press("Escape");
  const transfer = await page.evaluateHandle(async encoded => {
    const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
    const root = await navigator.storage.getDirectory();
    const handle = await root.getFileHandle("prototype.fig", { create: true });
    const writer = await handle.createWritable();
    await writer.write(bytes);
    await writer.close();
    DataTransferItem.prototype.getAsFileSystemHandle = async () => handle;
    const transfer = new DataTransfer();
    transfer.items.add(await handle.getFile());
    return transfer;
  }, (await readFile(fixture)).toString("base64"));
  await page.locator("#figstudio").dispatchEvent("drop", { dataTransfer: transfer });
  await expect.poll(async () => (await status(page)).tabs, { timeout: 60_000 }).toBe(1);
  await expect.poll(async () => (await status(page)).busy).toBe(false);
  await page.keyboard.press("Escape");
  const canvas = await page.screenshot({ path: testInfo.outputPath("1-canvas.png") });

  await page.mouse.move(640, 400);
  await page.keyboard.press("Shift+Space");
  await expect.poll(async () => (await status(page)).preview).toBe(true);
  await expect.poll(async () => (await status(page)).busy).toBe(false);
  await page.mouse.move(5, 795);
  await page.waitForTimeout(1000);
  const first = await page.screenshot({ path: testInfo.outputPath("2-preview-first.png") });
  expect((await compare(page, canvas, first)).changed, "Preview shows something else than the canvas").toBeGreaterThan(5000);

  // A click on no link flashes the links: the bluer pixels are the link.
  await page.mouse.click(640, 30);
  const flash = await page.screenshot({ path: testInfo.outputPath("3-missed-click-flash.png") });
  const { box } = await compare(page, first, flash);
  expect(box, "the links flash after a missed click").not.toBeNull();
  expect((await status(page)).preview).toBe(true);
  await page.waitForTimeout(1200);

  await page.mouse.click((box.x0 + box.x1) / 2, (box.y0 + box.y1) / 2);
  await page.waitForTimeout(800);
  await expect.poll(async () => (await status(page)).busy).toBe(false);
  await page.mouse.move(5, 795);
  await page.waitForTimeout(800);
  const second = await page.screenshot({ path: testInfo.outputPath("4-after-link.png") });
  expect((await compare(page, first, second)).changed, "the link shows another screen").toBeGreaterThan(5000);

  await page.keyboard.press("Backspace");
  await page.waitForTimeout(800);
  await expect.poll(async () => (await status(page)).busy).toBe(false);
  await page.waitForTimeout(800);
  const back = await page.screenshot({ path: testInfo.outputPath("5-back.png") });
  expect((await compare(page, first, back)).changed, "Back shows the first screen again").toBeLessThan(2000);

  await page.keyboard.press("Escape");
  await expect.poll(async () => (await status(page)).preview).toBe(false);
  await page.waitForTimeout(800);
  await page.screenshot({ path: testInfo.outputPath("6-exited.png") });
});
