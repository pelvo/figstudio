import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const launcher = join(here, "..", "bin", "figstudio-mcp.js");
const root = join(here, "..", "..", "..");
const { KILL_AFTER_MS, PLATFORMS, TEST_HOLD_MAX_MS, locate, packageName } = createRequire(import.meta.url)(launcher);

test("every platform has a release build, and the release builds only these", () => {
  const workflow = readFileSync(join(root, ".github", "workflows", "release-please.yml"), "utf8");
  for (const [key, p] of Object.entries(PLATFORMS)) {
    assert.ok(workflow.includes(`target: ${p.target}`), `release-please.yml builds ${p.target}`);
    assert.ok(workflow.includes(`npm: ${key}`), `release-please.yml packs ${key}`);
    assert.equal(key, `${p.os}-${p.cpu}`, "the key is what npm reports as platform-arch");
  }
  const built = [...workflow.matchAll(/^\s+- target: (\S+)/gm)].map((m) => m[1]);
  assert.deepEqual(built.sort(), Object.values(PLATFORMS).map((p) => p.target).sort());
});

test("locate resolves the binary inside this platform's package", () => {
  const resolve = (spec) => `/nm/${spec}`;
  assert.equal(locate({ platform: "linux", arch: "x64", env: {}, resolve }),
    "/nm/@pelvo/figstudio-mcp-linux-x64/bin/figstudio-mcp");
  assert.equal(locate({ platform: "win32", arch: "x64", env: {}, resolve }),
    "/nm/@pelvo/figstudio-mcp-win32-x64/bin/figstudio-mcp.exe");
  assert.equal(locate({ platform: "darwin", arch: "arm64", env: { FIGSTUDIO_MCP_BINARY: "/x/mcp" }, resolve }), "/x/mcp");
});

test("locate explains an unsupported computer and a skipped optional package", () => {
  assert.throws(() => locate({ platform: "freebsd", arch: "x64", env: {}, resolve: () => "" }),
    /no prebuilt binary for freebsd-x64.*cargo install --locked --git/s);
  const missing = () => { throw new Error("Cannot find module"); };
  assert.throws(() => locate({ platform: "linux", arch: "arm64", env: {}, resolve: missing }),
    /@pelvo\/figstudio-mcp-linux-arm64 is not installed.*--omit=optional/s);
});

test("the launcher passes argv, stdin, stdout and the exit code through", { skip: process.platform === "win32" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "figstudio-npm-"));
  const fake = join(dir, "fake-mcp");
  writeFileSync(fake, '#!/bin/sh\nprintf "args:%s|" "$*"\ncat\nexit 7\n');
  chmodSync(fake, 0o755);
  const res = spawnSync(process.execPath, [launcher, "--browser-proxy", "a b"], {
    input: '{"jsonrpc":"2.0","id":1,"method":"ping"}\n',
    env: { ...process.env, FIGSTUDIO_MCP_BINARY: fake },
    encoding: "utf8",
    timeout: 10_000,
  });
  assert.equal(res.error, undefined, "the launcher finished");
  assert.equal(res.status, 7);
  assert.equal(res.stdout, 'args:--browser-proxy a b|{"jsonrpc":"2.0","id":1,"method":"ping"}\n');
});

// Runs the launcher on a fake binary that prints "ready <pid>" and sends it `signals` 200 ms
// apart, while the launcher is held for 300 ms right after spawn() (the window a handler
// installed too late would leave open). Returns its exit code and the time from the first
// signal to its exit. A launcher that never exits is reported as "stuck" after 8 s and
// killed, and the fake is always killed afterwards, so a regression fails the test instead
// of hanging the run or leaving a signal-ignoring process behind.
async function signalled(script, signals) {
  const dir = mkdtempSync(join(tmpdir(), "figstudio-npm-"));
  const fake = join(dir, "fake-mcp");
  writeFileSync(fake, script);
  chmodSync(fake, 0o755);
  const child = spawn(process.execPath, [launcher], {
    env: { ...process.env, FIGSTUDIO_MCP_BINARY: fake, FIGSTUDIO_MCP_TEST_HOLD_AFTER_SPAWN_MS: "300" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  const exited = new Promise((resolve) => child.once("exit", (code) => resolve(code)));
  const ready = String(await new Promise((resolve) => child.stdout.once("data", resolve)));
  const fakePid = Number(ready.trim().split(" ")[1]);
  const start = Date.now();
  for (const [i, signal] of signals.entries()) {
    if (i) await new Promise((resolve) => setTimeout(resolve, 200));
    child.kill(signal);
  }
  let deadline;
  const stuck = new Promise((resolve) => { deadline = setTimeout(resolve, 8000, "stuck"); });
  const code = await Promise.race([exited, stuck]);
  clearTimeout(deadline);
  if (code === "stuck") child.kill("SIGKILL");
  try { process.kill(fakePid, "SIGKILL"); } catch { /* already gone (ESRCH) */ }
  child.stdout.destroy();
  return { code, ms: Date.now() - start };
}

// The signal tests below catch a handler installed after the test hold, but not one in
// the gap between spawn() and the hold, so the source order is checked here: every
// handler is installed before spawn(), and the hold is the next statement after it.
test("signal handlers are installed before spawn() and the hold follows it", () => {
  const source = readFileSync(launcher, "utf8");
  const body = source.slice(source.indexOf("function run("), source.indexOf("module.exports"));
  const code = body.split("\n").filter((line) => !line.trim().startsWith("//")).join("\n");
  const spawnAt = code.indexOf("child = spawn(");
  assert.ok(spawnAt > 0, "run() calls spawn()");
  const afterSpawn = code.slice(code.indexOf(";", spawnAt) + 1).trimStart();
  assert.match(afterSpawn, /^const holdMs = /, "the test hold is the first statement after spawn()");
  assert.ok(code.lastIndexOf("process.on(") < spawnAt, "no process.on after spawn()");
  assert.ok(code.indexOf("process.on(") >= 0, "the signal handlers exist");
});

test("the test hold is capped", { skip: process.platform === "win32", timeout: 10_000 }, () => {
  const dir = mkdtempSync(join(tmpdir(), "figstudio-npm-"));
  const fake = join(dir, "fake-mcp");
  writeFileSync(fake, "#!/bin/sh\nexit 0\n");
  chmodSync(fake, 0o755);
  const start = Date.now();
  const res = spawnSync(process.execPath, [launcher], {
    env: { ...process.env, FIGSTUDIO_MCP_BINARY: fake, FIGSTUDIO_MCP_TEST_HOLD_AFTER_SPAWN_MS: "5000" },
    timeout: 8_000,
  });
  const ms = Date.now() - start;
  assert.equal(res.status, 0);
  assert.equal(TEST_HOLD_MAX_MS, 300, "the signal tests' hold");
  // The hold runs (5000 asked), and only for the cap plus process start-up (about 25 ms;
  // a literal ceiling, so a looser cap elsewhere in the launcher still fails).
  assert.ok(ms >= 300 && ms < 300 + 400, `held for the cap (${ms} ms)`);
});

const DIES = "#!/bin/sh\necho ready $$\nexec sleep 30\n";
const IGNORES = "#!/bin/sh\ntrap '' INT TERM HUP\necho ready $$\nwhile :; do sleep 1; done\n";

test("a signal reaches the binary and the launcher exits with it", { skip: process.platform === "win32", timeout: 10_000 }, async () => {
  const { code, ms } = await signalled(DIES, ["SIGTERM"]);
  assert.equal(code, 128 + 15, "SIGTERM");
  assert.ok(ms < KILL_AFTER_MS, `exited at once (${ms} ms)`);
});

test("a second signal kills a binary that ignored the first", { skip: process.platform === "win32", timeout: 10_000 }, async () => {
  const { code, ms } = await signalled(IGNORES, ["SIGTERM", "SIGTERM"]);
  assert.equal(code, 128 + 9, "SIGKILL");
  assert.ok(ms < KILL_AFTER_MS, `the second signal, not the timer (${ms} ms)`);
});

test("a binary still running after KILL_AFTER_MS is killed", { skip: process.platform === "win32", timeout: 10_000 }, async () => {
  const { code, ms } = await signalled(IGNORES, ["SIGTERM"]);
  assert.equal(code, 128 + 9, "SIGKILL");
  assert.ok(ms >= KILL_AFTER_MS - 50 && ms < KILL_AFTER_MS + 3000, `after the grace time (${ms} ms)`);
});

test("a missing binary is reported on stderr with exit 1", () => {
  const res = spawnSync(process.execPath, [launcher], {
    env: { ...process.env, FIGSTUDIO_MCP_BINARY: join(tmpdir(), "figstudio-no-such-binary") },
    encoding: "utf8",
    timeout: 10_000,
  });
  assert.equal(res.status, 1);
  assert.match(res.stderr, /could not start/);
  assert.equal(res.stdout, "");
});

test("npm-pack builds installable packages with no install scripts", async () => {
  const { assemble } = await import(join(root, "scripts", "npm-pack.mjs"));
  const dir = mkdtempSync(join(tmpdir(), "figstudio-pack-"));
  const bin = join(dir, "built");
  writeFileSync(bin, "binary");
  const out = join(dir, "out");
  assemble({ version: "1.2.3", out, bins: { "linux-x64": bin, "win32-x64": bin }, pack: false });

  const main = JSON.parse(readFileSync(join(out, "figstudio-mcp", "package.json"), "utf8"));
  assert.equal(main.name, "figstudio-mcp");
  assert.equal(main.version, "1.2.3");
  assert.deepEqual(Object.keys(main.optionalDependencies).sort(),
    Object.keys(PLATFORMS).map(packageName).sort());
  assert.ok(Object.values(main.optionalDependencies).every((v) => v === "1.2.3"));
  assert.equal(main.bin["figstudio-mcp"], "bin/figstudio-mcp.js");

  const linux = JSON.parse(readFileSync(join(out, "figstudio-mcp-linux-x64", "package.json"), "utf8"));
  assert.deepEqual([linux.name, linux.os, linux.cpu], ["@pelvo/figstudio-mcp-linux-x64", ["linux"], ["x64"]]);
  if (process.platform !== "win32") {
    assert.ok(statSync(join(out, "figstudio-mcp-linux-x64", "bin", "figstudio-mcp")).mode & 0o100, "executable");
  }
  assert.ok(statSync(join(out, "figstudio-mcp-win32-x64", "bin", "figstudio-mcp.exe")).isFile());
  for (const pkg of [main, linux]) assert.equal(pkg.scripts, undefined, `${pkg.name} has no scripts`);
});
