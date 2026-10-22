import { test, expect } from "@playwright/test";
import { readFile, writeFile } from "node:fs/promises";

// Supply a generated PUBLIC document with stored emoji markers, never a private file.
const fixture = process.env.FIGSTUDIO_EMOJI_FIXTURE;
const baseURL = process.env.FIGSTUDIO_BASE_URL ?? "http://127.0.0.1:8765";

for (const renderer of ["cpu", "gpu"]) {
  test(`stored canvas emoji have colour on render workers (${renderer})`, async ({ page }, testInfo) => {
    test.skip(!fixture, "set FIGSTUDIO_EMOJI_FIXTURE to the generated public smoke input");
    test.setTimeout(120_000);
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto(`${baseURL}/?renderer=${renderer}&threads=2`);
    await expect(page.locator("#status")).toHaveText("", { timeout: 60_000 });
    await expect(page.getByRole("textbox")).toBeAttached({ timeout: 60_000 });
    // The welcome sheet has coloured artwork inside the crop: dismiss it, including its fade.
    await page.locator("#figstudio").focus();
    await page.keyboard.press("Escape");
    await page.waitForTimeout(600);
    const transfer = await page.evaluateHandle(async encoded => {
      const bytes = Uint8Array.from(atob(encoded), c => c.charCodeAt(0));
      // Real OPFS handle lets the app keep the drop in Recents, as a picked file would.
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
    await page.mouse.move(8, 8); // Keep hover outlines out of the pixel evidence.
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    // Exclude sidebar, first-run edge tips, toolbars and status pills.
    const screenshot = () => page.screenshot({ clip: { x: 300, y: 220, width: 650, height: 420 } });
    const colouredPixels = async () => {
      const encoded = (await screenshot()).toString("base64");
      return page.evaluate(async encoded => {
        const blob = await (await fetch(`data:image/png;base64,${encoded}`)).blob();
        const bitmap = await createImageBitmap(blob);
        const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
        const ctx = canvas.getContext("2d");
        ctx.drawImage(bitmap, 0, 0);
        const rgba = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
        let count = 0;
        for (let i = 0; i < rgba.length; i += 4) {
          if (rgba[i + 3] && Math.max(rgba[i], rgba[i + 1], rgba[i + 2]) - Math.min(rgba[i], rgba[i + 1], rgba[i + 2]) > 40) count++;
        }
        return count;
      }, encoded);
    };
    await expect.poll(colouredPixels, { timeout: 45_000 }).toBeGreaterThan(300);
    console.log(`${renderer}: ${await colouredPixels()} non-gray canvas pixels`);
    expect(await page.evaluate(() => window.figstudioFatal())).toBe(false);
    expect(errors).toEqual([]);
    const thumbnail = () => page.evaluate(async () => {
      if (!(await indexedDB.databases()).some(db => db.name === "figstudio-recents")) return null;
      const png = await new Promise((resolve, reject) => {
        const open = indexedDB.open("figstudio-recents", 1);
        open.onerror = () => reject(open.error);
        open.onsuccess = () => {
          const db = open.result;
          const request = db.transaction("thumbs").objectStore("thumbs").getAll();
          request.onsuccess = () => { resolve(request.result[0] ?? null); db.close(); };
          request.onerror = () => { reject(request.error); db.close(); };
        };
      });
      if (!png) return null;
      const bitmap = await createImageBitmap(new Blob([png], { type: "image/png" }));
      const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d");
      ctx.drawImage(bitmap, 0, 0);
      const rgba = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let colours = 0;
      for (let i = 0; i < rgba.length; i += 4) {
        if (rgba[i + 3] && Math.max(rgba[i], rgba[i + 1], rgba[i + 2]) - Math.min(rgba[i], rgba[i + 1], rgba[i + 2]) > 40) colours++;
      }
      return { colours, png: Array.from(png) };
    });
    await expect.poll(async () => (await thumbnail())?.colours ?? -1, { timeout: 45_000 }).toBeGreaterThan(300);
    const thumb = await thumbnail();
    console.log(`${renderer}: ${thumb.colours} non-gray Recents thumbnail pixels`);
    await writeFile(testInfo.outputPath(`recents-emoji-${renderer}.png`), Buffer.from(thumb.png));
    await page.screenshot({ path: testInfo.outputPath(`canvas-emoji-${renderer}.jpg`), type: "jpeg", quality: 85 });
  });
}
