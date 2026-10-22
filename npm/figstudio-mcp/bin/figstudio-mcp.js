#!/usr/bin/env node
// Runs the prebuilt figstudio-mcp binary for this computer, passing stdin, stdout and
// stderr straight through (MCP over stdio). The binaries ship in the optional packages
// listed in PLATFORMS; npm installs only the one whose `os` and `cpu` match, so no
// install script is needed.
"use strict";

const SCOPE = "@pelvo";

/** npm platform key (`${process.platform}-${process.arch}`) -> Rust target and npm fields. */
const PLATFORMS = {
  "linux-x64": { target: "x86_64-unknown-linux-musl", os: "linux", cpu: "x64" },
  "linux-arm64": { target: "aarch64-unknown-linux-musl", os: "linux", cpu: "arm64" },
  "darwin-arm64": { target: "aarch64-apple-darwin", os: "darwin", cpu: "arm64" },
  "darwin-x64": { target: "x86_64-apple-darwin", os: "darwin", cpu: "x64" },
  "win32-x64": { target: "x86_64-pc-windows-msvc", os: "win32", cpu: "x64" },
};

/** How long a child may run on after a forwarded signal before it gets SIGKILL. */
const KILL_AFTER_MS = 2000;

const CARGO_HINT =
  "Build it instead with: cargo install --locked --git https://github.com/pelvo/figstudio figstudio-mcp";

function packageName(key) {
  return `${SCOPE}/figstudio-mcp-${key}`;
}

function binaryName(os) {
  return os === "win32" ? "figstudio-mcp.exe" : "figstudio-mcp";
}

/** Path of the binary to run. FIGSTUDIO_MCP_BINARY overrides the lookup. */
function locate({
  platform = process.platform,
  arch = process.arch,
  env = process.env,
  resolve = require.resolve,
} = {}) {
  if (env.FIGSTUDIO_MCP_BINARY) return env.FIGSTUDIO_MCP_BINARY;
  const key = `${platform}-${arch}`;
  if (!Object.hasOwn(PLATFORMS, key)) {
    throw new Error(
      `no prebuilt binary for ${key} (there are: ${Object.keys(PLATFORMS).join(", ")}). ${CARGO_HINT}`,
    );
  }
  const pkg = packageName(key);
  try {
    return resolve(`${pkg}/bin/${binaryName(platform)}`);
  } catch {
    throw new Error(
      `${pkg} is not installed. It is an optional dependency of figstudio-mcp: install ` +
        `again without --no-optional or --omit=optional. ${CARGO_HINT}`,
    );
  }
}

function run(argv = process.argv.slice(2)) {
  const fs = require("node:fs");
  const os = require("node:os");
  const { spawn } = require("node:child_process");
  let bin;
  try {
    bin = locate();
  } catch (error) {
    process.stderr.write(`figstudio-mcp: ${error.message}\n`);
    process.exit(1);
  }
  // Some package managers drop the executable bit; restore it once (best effort).
  if (process.platform !== "win32") {
    try {
      if ((fs.statSync(bin).mode & 0o111) === 0) fs.chmodSync(bin, 0o755);
    } catch {
      // A read-only store keeps its own modes; spawn reports a real problem below.
    }
  }
  // These handlers turn off Node's own exit on the signals, so the launcher waits for the
  // child. The first signal is passed on; a second one, or a child still running
  // KILL_AFTER_MS later, gets SIGKILL, so a host that gives up and kills the launcher does
  // not leave a stuck child behind. They are installed before spawn(): a signal landing
  // between the two would otherwise get Node's default action, killing the launcher and
  // orphaning the child. Their callbacks run after this function returns, so `child` is
  // set by then; the check only guards that order.
  let child;
  let forwarded = false;
  const forward = (signal) => {
    if (!child || child.exitCode !== null || child.signalCode !== null) return;
    if (forwarded) {
      child.kill("SIGKILL");
      return;
    }
    forwarded = true;
    child.kill(signal);
    setTimeout(() => child.kill("SIGKILL"), KILL_AFTER_MS).unref();
  };
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) process.on(signal, forward);
  child = spawn(bin, argv, { stdio: "inherit", windowsHide: true });
  // Test hook, never set in use: launcher.test.mjs holds the launcher here so its signals
  // land in the window a handler installed after spawn() would leave open.
  const holdMs = Number(process.env.FIGSTUDIO_MCP_TEST_HOLD_AFTER_SPAWN_MS) || 0;
  if (holdMs > 0) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, holdMs);
  child.on("error", (error) => {
    process.stderr.write(`figstudio-mcp: could not start ${bin}: ${error.message}\n`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    process.exit(signal ? 128 + (os.constants.signals[signal] || 0) : (code ?? 1));
  });
}

module.exports = { SCOPE, PLATFORMS, KILL_AFTER_MS, packageName, binaryName, locate };

if (require.main === module) run();
