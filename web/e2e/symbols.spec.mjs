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
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  expect(fonts.size).toBe(0);
  const buffer = await readFile(new URL("../../tests/fixtures/corners.fig", import.meta.url));
  const open = async name => {
    await page.locator("#figstudio").click();
    const chooser = page.waitForEvent("filechooser", { timeout: 15_000 });
    await page.keyboard.press(process.platform === "darwin" ? "Meta+o" : "Control+o");
    await (await chooser).setFiles({ name, mimeType: "application/octet-stream", buffer });
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
