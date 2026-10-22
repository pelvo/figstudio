import { test, expect } from "@playwright/test";
import { pathToFileURL } from "node:url";
import { join } from "node:path";

// Opens the sample compare report written by crates/diff/tests/sample_report.rs:
//   FIGSTUDIO_SAMPLE_REPORT=/tmp/sample cargo test -p figstudio-diff --test sample_report
//   FIGSTUDIO_SAMPLE_REPORT=/tmp/sample npx playwright test compare-report.spec.mjs
// Screenshots are attached to the run for a look by eye; the checks cover layout only.
const dir = process.env.FIGSTUDIO_SAMPLE_REPORT;
test.skip(!dir, "set FIGSTUDIO_SAMPLE_REPORT to the sample report folder");

const open = async (page, width) => {
  await page.setViewportSize({ width, height: 900 });
  const errors = [];
  page.on("console", message => message.type() === "error" && errors.push(message.text()));
  await page.goto(pathToFileURL(join(dir, "report.html")).href);
  await page.waitForLoadState("load");
  return errors;
};

const noSideScroll = page =>
  page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth);

for (const width of [1280, 390]) {
  test(`report at ${width} px loads every picture and does not scroll sideways`, async ({ page }, info) => {
    const errors = await open(page, width);
    // Pictures are lazy and some sit in closed sections: load them all before checking.
    const broken = await page.evaluate(async () => {
      document.querySelectorAll("details").forEach(details => { details.open = true; });
      const images = [...document.images];
      images.forEach(image => { image.loading = "eager"; });
      await Promise.all(images.map(image => image.decode().catch(() => {})));
      return images.filter(image => image.naturalWidth === 0).map(image => image.src);
    });
    expect(broken).toEqual([]);
    expect(await noSideScroll(page)).toBe(true);
    expect(errors).toEqual([]);
    await info.attach(`report-${width}.png`, { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
  });
}

test("overlay shows After and reveals Before on hover", async ({ page }, info) => {
  await open(page, 1280);
  await page.locator('label[for="view-overlay"]').click();
  const card = page.locator(".cmp").first();
  const before = card.locator(".bf");
  await page.mouse.move(0, 0);
  await expect(before).toHaveCSS("opacity", "0");
  await card.hover();
  await expect(before).toHaveCSS("opacity", "1");
  await info.attach("overlay.png", { body: await card.screenshot(), contentType: "image/png" });
});
