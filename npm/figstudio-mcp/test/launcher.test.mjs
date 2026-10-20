import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const launcher = join(here, "..", "bin", "figstudio-mcp.js");
const root = join(here, "..", "..", "..");
const { PLATFORMS, locate, packageName } = createRequire(import.meta.url)(launcher);

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
