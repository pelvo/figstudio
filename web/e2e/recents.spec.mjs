import { test, expect } from "@playwright/test";
import http from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// The browser Recents store (apps/figstudio/js/recents.js) in Chromium, with real
// IndexedDB and real file handles: handles from the origin's private file system (OPFS)
// stand in for picked ones (Playwright cannot drive showOpenFilePicker). No app build or
// relay needed: the module is served on its own.
const moduleFile = fileURLToPath(new URL("../../apps/figstudio/js/recents.js", import.meta.url));

let server;
let base;

test.beforeAll(async () => {
  const source = await readFile(moduleFile);
  server = http.createServer((req, res) => {
    if (req.url === "/recents.js") {
      res.setHeader("content-type", "text/javascript");
      res.end(source);
    } else {
      res.setHeader("content-type", "text/html");
      res.end("<!doctype html><title>recents</title>");
    }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  // localhost is a secure context: OPFS and IndexedDB work.
  base = `http://localhost:${server.address().port}`;
});

test.afterAll(() => server.close());

test.beforeEach(async ({ page }) => {
  await page.goto(`${base}/`);
});

// Runs `body` in the page with the module as `m` and a helper `opfs(name, text)` that
// writes a file in OPFS and returns its handle.
const inPage = (page, body, arg) =>
  page.evaluate(
    async ({ body, arg }) => {
      const m = await import("/recents.js");
      const root = await navigator.storage.getDirectory();
      const opfs = async (name, text) => {
        const h = await root.getFileHandle(name, { create: true });
        const w = await h.createWritable();
        await w.write(text);
        await w.close();
        return h;
      };
      // eslint-disable-next-line no-new-func
      return new Function("m", "root", "opfs", "arg", `return (async () => {${body}})()`)(
        m, root, opfs, arg,
      );
    },
    { body, arg },
  );

test("the list is kept, and starts empty", async ({ page }) => {
  expect(await inPage(page, "return await m.loadList();")).toBeNull();
  await inPage(page, `await m.saveList('[{"path":"a.fig","opened":1}]');`);
  await page.reload();
  expect(await inPage(page, "return await m.loadList();")).toBe('[{"path":"a.fig","opened":1}]');
});

test("one file keeps one handle number; another file gets its own", async ({ page }) => {
  const ids = await inPage(page, `
    const a = await opfs("a.fig", "one");
    const b = await opfs("b.fig", "two");
    const first = await m.keepHandle(a);
    const again = await m.keepHandle(await root.getFileHandle("a.fig"));
    const other = await m.keepHandle(b);
    return { first, again, other };
  `);
  expect(ids.again).toBe(ids.first);
  expect(ids.other).not.toBe(ids.first);
});

test("a kept handle opens its file after a reload, and says when it is gone", async ({ page }) => {
  const id = await inPage(page, `return await m.keepHandle(await opfs("c.fig", "three"));`);
  await page.reload();
  const opened = await inPage(page, `
    const r = await m.openHandle(arg);
    return r.file ? { name: r.file.name, text: await r.file.text() } : r;
  `, id);
  expect(opened).toEqual({ name: "c.fig", text: "three" });
  await inPage(page, `await root.removeEntry("c.fig");`);
  expect(await inPage(page, "return await m.openHandle(arg);", id)).toEqual({ error: "missing" });
  await inPage(page, "await m.forgetHandle(arg);", id);
  expect(await inPage(page, "return await m.openHandle(arg);", id)).toEqual({ error: "gone" });
});

test("pictures are kept as bytes and deleted", async ({ page }) => {
  const back = await inPage(page, `
    await m.putThumb("h1", new Uint8Array([137, 80, 78, 71]));
    return Array.from(await m.getThumb("h1"));
  `);
  expect(back).toEqual([137, 80, 78, 71]);
  expect(await inPage(page, `await m.deleteThumb("h1"); return await m.getThumb("h1");`))
    .toBeNull();
});

test("without the File System Access API the store says so", async ({ page }) => {
  expect(await inPage(page, "return m.keepsHandles();")).toBe(true);
  await page.addInitScript(() => { delete window.showOpenFilePicker; });
  await page.reload();
  expect(await inPage(page, "return m.keepsHandles();")).toBe(false);
});
