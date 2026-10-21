import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// Recents in the built browser app (scripts/web-build.sh, then
// `python3 scripts/web-serve.py web/dist 8765`):
//   FIGSTUDIO_WEB_URL=http://127.0.0.1:8765/ npx playwright test recents-app.spec.mjs
// egui draws to a canvas, so the page is driven by position (1280×800: the first card's
// picture is at 385,165) and checked through what the app keeps in IndexedDB.
const baseURL = process.env.FIGSTUDIO_WEB_URL;
const fixture = fileURLToPath(new URL("../../tests/fixtures/structure.fig", import.meta.url));
const firstCard = { x: 385, y: 165 };
const open = process.platform === "darwin" ? "Meta+o" : "Control+o";
const close = process.platform === "darwin" ? "Meta+w" : "Control+w";

test.skip(!baseURL, "set FIGSTUDIO_WEB_URL to a served web/dist");
test.setTimeout(120_000);

// What the app keeps: the list, and the keys of the pictures and handles.
const stored = (page) => page.evaluate(async () => {
  if (!(await indexedDB.databases()).some((d) => d.name === "figstudio-recents")) return null;
  const db = await new Promise((resolve, reject) => {
    const req = indexedDB.open("figstudio-recents");
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  const all = (store, how) => new Promise((resolve, reject) => {
    const req = db.transaction(store).objectStore(store)[how]();
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  const [list] = await all("list", "getAll");
  const out = {
    list: list ? JSON.parse(list) : [],
    thumbs: await all("thumbs", "getAllKeys"),
    handles: await all("handles", "getAllKeys"),
  };
  db.close();
  return out;
});

// Loads the app and puts the welcome sheet away. The app opens its store as it starts.
const start = async (page) => {
  await page.goto(baseURL);
  await expect(page.locator("#figstudio")).toBeVisible();
  await expect.poll(() => stored(page), { timeout: 60_000 }).toBeTruthy();
  await page.locator("#figstudio").click({ position: { x: 900, y: 600 } });
  await page.keyboard.press("Escape");
};

// Does `act` until `shown()` (the first frames may not take keys yet, and the saved list
// arrives a moment after the page).
const until = async (page, act, shown) => {
  for (let i = 0; i < 10; i++) {
    const done = shown();
    await act();
    if (await done) return;
    await page.keyboard.press("Escape");
  }
  throw new Error("nothing happened");
};
const pressOpen = (page, shown) => until(page, () => page.keyboard.press(open), shown);
const clickCard = (page, shown) =>
  until(page, () => page.mouse.click(firstCard.x, firstCard.y), shown);
const chooser = (page) =>
  page.waitForEvent("filechooser", { timeout: 3000 }).then((c) => c.setFiles(fixture).then(() => true), () => false);

test.describe("without file handles (Firefox, Safari)", () => {
  test.beforeEach(async ({ context }) => {
    await context.addInitScript(() => { delete window.showOpenFilePicker; });
  });

  test("a picked file is listed with its picture, and its card asks for the file again", async ({ page }) => {
    await start(page);
    await pressOpen(page, () => chooser(page));
    await expect.poll(async () => (await stored(page)).thumbs, { timeout: 60_000 })
      .toEqual(["n:structure.fig"]);
    const { list, handles } = await stored(page);
    expect(list.map((e) => [e.path, e.handle])).toEqual([["structure.fig", undefined]]);
    expect(handles).toEqual([]);

    // Kept across a reload; with no handle the card shows the picker.
    await page.keyboard.press(close);
    await page.reload();
    await start(page);
    await clickCard(page, () =>
      page.waitForEvent("filechooser", { timeout: 3000 }).then(() => true, () => false));
  });
});

test.describe("with file handles (Chrome, Edge)", () => {
  // The picker hands out a handle from the origin's private file system (Playwright cannot
  // drive the real one), and handle reads are counted.
  test.beforeEach(async ({ context }) => {
    await context.addInitScript(() => {
      window.pickerCalls = [];
      window.reads = 0;
      const getFile = FileSystemFileHandle.prototype.getFile;
      FileSystemFileHandle.prototype.getFile = function () {
        window.reads += 1;
        return getFile.call(this);
      };
      window.showOpenFilePicker = async (options) => {
        window.pickerCalls.push(options?.startIn ? "in its folder" : "plain");
        const root = await navigator.storage.getDirectory();
        return [await root.getFileHandle("structure.fig")];
      };
    });
  });

  test("a card opens its file from the handle, and a moved file opens the picker in its place", async ({ page }) => {
    await start(page);
    const bytes = (await readFile(fixture)).toString("base64");
    await page.evaluate(async (b64) => {
      const root = await navigator.storage.getDirectory();
      const file = await root.getFileHandle("structure.fig", { create: true });
      const out = await file.createWritable();
      await out.write(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)));
      await out.close();
    }, bytes);
    await pressOpen(page, () =>
      expect.poll(() => page.evaluate(() => window.pickerCalls.length), { timeout: 3000 })
        .toBe(1).then(() => true, () => false));
    await expect.poll(async () => (await stored(page)).thumbs, { timeout: 60_000 }).toEqual(["h1"]);
    const first = await stored(page);
    expect(first.list.map((e) => [e.path, e.handle])).toEqual([["structure.fig", 1]]);
    expect(first.handles).toEqual([1]);

    // After a reload the card reads the file through its handle: no picker, and the file is
    // listed again as just opened (the time is in seconds).
    await page.keyboard.press(close);
    await page.reload();
    await start(page);
    await page.waitForTimeout(1100);
    await clickCard(page, () =>
      expect.poll(() => page.evaluate(() => window.reads), { timeout: 3000 })
        .toBeGreaterThan(0).then(() => true, () => false));
    await expect.poll(async () => (await stored(page)).list[0].opened, { timeout: 60_000 })
      .toBeGreaterThan(first.list[0].opened);
    expect(await page.evaluate(() => window.pickerCalls)).toEqual([]);
    expect(await page.evaluate(() => window.reads)).toBeGreaterThan(0);

    // Moved away: the picker starts where the file was.
    await page.evaluate(async () => {
      await (await navigator.storage.getDirectory()).removeEntry("structure.fig");
    });
    await page.keyboard.press(close);
    await clickCard(page, () =>
      expect.poll(() => page.evaluate(() => window.pickerCalls[0]), { timeout: 3000 })
        .toBe("in its folder").then(() => true, () => false));
  });
});

// Two files of one name: picked together, one load runs and the other is let go. The card
// must keep the handle of the file that opened (review of #47).
test("of two picked files with one name, the card keeps the handle of the one that opened", async ({ context, page }) => {
  await context.addInitScript(() => {
    window.picks = 0;
    window.showOpenFilePicker = async () => {
      window.picks += 1;
      const root = await navigator.storage.getDirectory();
      const one = await root.getDirectoryHandle("one");
      const two = await root.getDirectoryHandle("two");
      return [await one.getFileHandle("structure.fig"), await two.getFileHandle("structure.fig")];
    };
  });
  await start(page);
  const write = async (dir, file) => {
    const b64 = (await readFile(fileURLToPath(new URL(`../../tests/fixtures/${file}`, import.meta.url)))).toString("base64");
    await page.evaluate(async ([dir, b64]) => {
      const root = await navigator.storage.getDirectory();
      const d = await root.getDirectoryHandle(dir, { create: true });
      const out = await (await d.getFileHandle("structure.fig", { create: true })).createWritable();
      await out.write(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)));
      await out.close();
    }, [dir, b64]);
  };
  await write("one", "structure.fig");
  await write("two", "vector.fig");
  await pressOpen(page, () =>
    expect.poll(() => page.evaluate(() => window.picks), { timeout: 3000 })
      .toBe(1).then(() => true, () => false));
  await expect.poll(async () => (await stored(page)).list.length, { timeout: 30_000 }).toBe(1);
  // The tab shows structure.fig (the first), so the card's handle must read that file.
  const size = (await readFile(fixture)).length;
  const kept = await page.evaluate(async () => {
    const db = await new Promise((resolve) => {
      const req = indexedDB.open("figstudio-recents");
      req.onsuccess = () => resolve(req.result);
    });
    const get = (store, key) => new Promise((resolve) => {
      const req = db.transaction(store).objectStore(store).get(key);
      req.onsuccess = () => resolve(req.result);
    });
    const list = JSON.parse(await get("list", "entries"));
    const handle = await get("handles", list[0].handle);
    return (await handle.getFile()).size;
  });
  expect(kept).toBe(size);
});

// Relocating a moved card: only a file its own picker returns, with a handle, takes the
// card's place (reviews of #47). The fake picker answers from `window.answers`: "top"
// (OPFS structure.fig), "other" (other/structure.fig), "cancel" or "refuse".
test.describe("relocating a moved card", () => {
  test.beforeEach(async ({ context }) => {
    await context.addInitScript(() => {
      window.answers = [];
      window.calls = 0;
      window.showOpenFilePicker = async () => {
        window.calls += 1;
        const next = window.answers.shift();
        if (next === "cancel") throw new DOMException("cancelled", "AbortError");
        if (next === "refuse") throw new DOMException("refused", "SecurityError");
        const root = await navigator.storage.getDirectory();
        const dir = next === "other" ? await root.getDirectoryHandle("other") : root;
        return [await dir.getFileHandle("structure.fig")];
      };
    });
  });

  // Opens structure.fig with a handle, closes it and deletes it: its card is missing next.
  const moved = async (page) => {
    await start(page);
    const b64 = (await readFile(fixture)).toString("base64");
    await page.evaluate(async (b64) => {
      const root = await navigator.storage.getDirectory();
      for (const dir of [root, await root.getDirectoryHandle("other", { create: true })]) {
        const out = await (await dir.getFileHandle("structure.fig", { create: true })).createWritable();
        await out.write(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)));
        await out.close();
      }
    }, b64);
    const answer = (list) => page.evaluate((list) => { window.answers = list; }, list);
    const called = (n) => () =>
      expect.poll(() => page.evaluate(() => window.calls), { timeout: 3000 })
        .toBeGreaterThanOrEqual(n).then(() => true, () => false);
    await answer(["top"]);
    await pressOpen(page, called(1));
    await expect.poll(async () => (await stored(page)).thumbs, { timeout: 60_000 }).toEqual(["h1"]);
    await page.keyboard.press(close);
    await page.evaluate(async () => {
      await (await navigator.storage.getDirectory()).removeEntry("structure.fig");
    });
    return { answer, called };
  };
  const handles = async (page) => (await stored(page)).list.map((e) => e.handle ?? null);

  test("a cancelled relocate leaves the card to a later open of that name", async ({ page }) => {
    const { answer, called } = await moved(page);
    await answer(["cancel"]);
    await clickCard(page, called(2));
    await answer(["other"]);
    await pressOpen(page, called(3));
    await expect.poll(() => handles(page), { timeout: 30_000 }).toEqual([2, 1]);
  });

  test("a relocate the browser refuses keeps the card and its handle", async ({ page }) => {
    const { answer } = await moved(page);
    // Refused in its folder and again without: the plain file input opens instead.
    await answer(["refuse", "refuse"]);
    await clickCard(page, () => chooser(page));
    await expect.poll(() => handles(page), { timeout: 30_000 }).toEqual([null, 1]);
  });
});
