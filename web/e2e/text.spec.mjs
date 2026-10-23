import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

const fixture = process.env.FIGSTUDIO_TEXT_FIXTURE;
const baseURL = process.env.FIGSTUDIO_BASE_URL ?? "http://127.0.0.1:8765";

for (const renderer of ["cpu", "gpu"]) {
  test(`outline-less text redraws after fonts arrive (${renderer})`, async ({ page }, testInfo) => {
    test.skip(!fixture, "set FIGSTUDIO_TEXT_FIXTURE to the generated synthetic input");
    test.setTimeout(120_000);
    let release;
    const fontGate = new Promise(resolve => { release = resolve; });
    await page.route("**/fonts/cjk-*.otf", async route => {
      await fontGate;
      await route.continue();
    });
    await page.goto(`${baseURL}/?renderer=${renderer}&threads=2`);
    await expect(page.locator("#status")).toHaveText("", { timeout: 60_000 });
    await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
    await page.locator("#figstudio").focus();
    await page.keyboard.press("Escape");
    await page.waitForTimeout(600);
    const transfer = await page.evaluateHandle(async encoded => {
      const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
      const root = await navigator.storage.getDirectory();
      const handle = await root.getFileHandle("synthetic.fig", { create: true });
      const writer = await handle.createWritable();
      await writer.write(bytes);
      await writer.close();
      DataTransferItem.prototype.getAsFileSystemHandle = async () => handle;
      const transfer = new DataTransfer();
      transfer.items.add(await handle.getFile());
      return transfer;
    }, (await readFile(fixture)).toString("base64"));
    await page.locator("#figstudio").dispatchEvent("drop", { dataTransfer: transfer });
    await page.mouse.move(8, 8);
    const countRed = encoded => page.evaluate(async encoded => {
        const blob = await (await fetch(`data:image/png;base64,${encoded}`)).blob();
        const bitmap = await createImageBitmap(blob);
        const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
        ctx.drawImage(bitmap, 0, 0);
        const rgba = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
        let count = 0;
        for (let i = 0; i < rgba.length; i += 4) {
          if (rgba[i] > rgba[i + 1] + 60 && rgba[i] > rgba[i + 2] + 60) count++;
        }
        return count;
      }, encoded);
    const redPixels = async () => countRed((await page.screenshot({
      clip: { x: 300, y: 220, width: 650, height: 420 },
    })).toString("base64"));
    const thumbPixels = async () => {
      const encoded = await page.evaluate(async () => {
        if (!(await indexedDB.databases()).some(db => db.name === "figstudio-recents")) return null;
        return new Promise((resolve, reject) => {
          const open = indexedDB.open("figstudio-recents", 1);
          open.onerror = () => reject(open.error);
          open.onsuccess = () => {
            const db = open.result;
            const request = db.transaction("thumbs").objectStore("thumbs").getAll();
            request.onsuccess = () => {
              const png = request.result[0];
              resolve(png ? btoa(String.fromCharCode(...png)) : null);
              db.close();
            };
            request.onerror = () => { reject(request.error); db.close(); };
          };
        });
      });
      return encoded ? countRed(encoded) : -1;
    };
    await expect.poll(() => page.evaluate(() => JSON.parse(window.figstudioStatus).tabs)).toBe(1);
    await expect.poll(() => page.evaluate(() => JSON.parse(window.figstudioStatus).busy)).toBe(false);
    expect(await redPixels()).toBe(0);
    await expect.poll(thumbPixels, { timeout: 45_000 }).toBe(0);
    await page.screenshot({ path: testInfo.outputPath("blocked.jpg"), type: "jpeg", quality: 85 });
    release();
    await expect.poll(redPixels, { timeout: 45_000 }).toBeGreaterThan(100);
    await expect.poll(thumbPixels, { timeout: 45_000 }).toBeGreaterThan(100);
    console.log(`${renderer}: canvas=${await redPixels()}, recents=${await thumbPixels()} red pixels; blocked=0`);
    expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
    await page.screenshot({ path: testInfo.outputPath("loaded.jpg"), type: "jpeg", quality: 85 });
  });
}
