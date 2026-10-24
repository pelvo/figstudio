import { test, expect } from "@playwright/test";
import { pathToFileURL } from "node:url";
import { join } from "node:path";

// Opens the sample compare report written by crates/diff/tests/sample_report.rs:
//   FIGSTUDIO_SAMPLE_REPORT=/tmp/sample cargo test -p figstudio-diff --test sample_report
//   FIGSTUDIO_SAMPLE_REPORT=/tmp/sample npx playwright test compare-report.spec.mjs
// Screenshots are attached to the run for a look by eye. The checks cover layout, console
// errors, and the overlay: Before shows on hover, on keyboard focus and while pressed.
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
  page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);

for (const width of [1280, 390]) {
  test(`report at ${width} px loads every picture and does not scroll sideways`, async ({ page }, info) => {
    const errors = await open(page, width);
    // Pictures are lazy and some sit in closed sections: open them, load every picture
    // eagerly and wait for each one's load or error event.
    const broken = await page.evaluate(async () => {
      document.querySelectorAll("details").forEach(details => { details.open = true; });
      const settle = image => new Promise(resolve => {
        if (image.complete) return resolve(image.naturalWidth > 0);
        image.addEventListener("load", () => resolve(true), { once: true });
        image.addEventListener("error", () => resolve(false), { once: true });
      });
      const images = [...document.images];
      images.forEach(image => { image.loading = "eager"; });
      const loaded = await Promise.all(images.map(settle));
      return images.filter((_, i) => !loaded[i]).map(image => image.src);
    });
    expect(broken).toEqual([]);
    expect(await noSideScroll(page)).toBe(true);
    expect(errors).toEqual([]);
    await info.attach(`report-${width}.png`, { body: await page.screenshot({ fullPage: true }), contentType: "image/png" });
  });
}

test("overlay shows After and reveals Before on hover and focus", async ({ page }, info) => {
  await open(page, 1280);
  await page.locator('label[for="view-overlay"]').click();
  const card = page.locator(".cmp").first();
  const before = card.locator(".bf");
  // `toHaveCSS` retries, so it waits out the short opacity transition.
  const hidden = async () => {
    await page.mouse.move(0, 0);
    await page.evaluate(() => document.activeElement?.blur());
    await expect(before).toHaveCSS("opacity", "0");
  };
  await hidden();
  await card.hover();
  await expect(before).toHaveCSS("opacity", "1");
  await info.attach("overlay.png", { body: await card.screenshot(), contentType: "image/png" });
  await hidden();
  await card.focus();
  await expect(before).toHaveCSS("opacity", "1");
  await hidden();
  // A real press also hovers and focuses the card, so `:active` can't be driven on its own;
  // check the overlay rule still lists it, for touch screens that have no hover.
  // Top-level rules only, so a move into `@media (hover: hover)` fails too.
  const pressed = await page.evaluate(() =>
    [...document.styleSheets]
      .flatMap(sheet => [...sheet.cssRules])
      .filter(rule => rule instanceof CSSStyleRule
        && rule.selectorText.split(",").map(part => part.trim().replace(/\s+/g, " "))
          .includes("#view-overlay:checked ~ .shell .cmp:active .bf"))
      .map(rule => rule.style.opacity));
  expect(pressed).toEqual(["1"]);
});
