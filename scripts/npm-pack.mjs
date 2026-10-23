#!/usr/bin/env node
// Assembles the npm packages of figstudio-mcp from built binaries:
//   <out>/figstudio-mcp/            the launcher (npm/figstudio-mcp) with the version set
//   <out>/figstudio-mcp-<key>/      one package per platform binary given with --bin
//
//   node scripts/npm-pack.mjs --version 0.1.0 --out dist/npm \
//     --bin linux-x64=target/x86_64-unknown-linux-musl/release/figstudio-mcp [--bin ...] \
//     [--require-all] [--pack]
//
// --require-all fails unless every platform in PLATFORMS has a binary (the release job).
// --pack also runs `npm pack` in each package and leaves the .tgz files in <out>.
import { chmodSync, copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const launcherDir = join(root, "npm", "figstudio-mcp");
const { PLATFORMS, packageName, binaryName } = createRequire(import.meta.url)(
  join(launcherDir, "bin", "figstudio-mcp.js"),
);

function parse(argv) {
  const opts = { bins: {}, requireAll: false, pack: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = () => {
      if (i + 1 >= argv.length) throw new Error(`${arg} needs a value`);
      return argv[++i];
    };
    if (arg === "--version") opts.version = value();
    else if (arg === "--out") opts.out = resolve(value());
    else if (arg === "--bin") {
      const [key, path] = value().split(/=(.*)/s);
      if (!Object.hasOwn(PLATFORMS, key)) throw new Error(`unknown platform ${key}`);
      if (!path || !existsSync(path)) throw new Error(`no binary at ${path} for ${key}`);
      opts.bins[key] = resolve(path);
    } else if (arg === "--require-all") opts.requireAll = true;
    else if (arg === "--pack") opts.pack = true;
    else throw new Error(`unknown argument ${arg}`);
  }
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(opts.version || "")) {
    throw new Error("--version must be a semver version such as 0.1.0");
  }
  if (!opts.out) throw new Error("--out is required");
  const missing = Object.keys(PLATFORMS).filter((key) => !opts.bins[key]);
  if (opts.requireAll && missing.length) throw new Error(`missing binaries: ${missing.join(", ")}`);
  return opts;
}

const shared = JSON.parse(readFileSync(join(launcherDir, "package.json"), "utf8"));
const license = join(root, "LICENSE-APACHE");

export function assemble({ version, out, bins, pack }) {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });
  const dirs = [];

  for (const [key, bin] of Object.entries(bins)) {
    const p = PLATFORMS[key];
    const dir = join(out, `figstudio-mcp-${key}`);
    mkdirSync(join(dir, "bin"), { recursive: true });
    const dest = join(dir, "bin", binaryName(p.os));
    copyFileSync(bin, dest);
    chmodSync(dest, 0o755);
    copyFileSync(license, join(dir, "LICENSE"));
    writeFileSync(join(dir, "README.md"),
      `# ${packageName(key)}\n\nThe \`figstudio-mcp\` binary for ${p.os} ${p.cpu} (\`${p.target}\`). ` +
      `It is installed by [figstudio-mcp](https://www.npmjs.com/package/figstudio-mcp); use that package.\n`);
    const pkg = {
      name: packageName(key),
      version,
      description: `figstudio-mcp binary for ${p.os} ${p.cpu} (${p.target}). Installed by figstudio-mcp.`,
      license: shared.license,
      homepage: shared.homepage,
      repository: shared.repository,
      os: [p.os],
      cpu: [p.cpu],
      files: ["bin/", "README.md", "LICENSE"],
      preferUnplugged: true,
    };
    writeFileSync(join(dir, "package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
    dirs.push(dir);
  }

  const main = join(out, "figstudio-mcp");
  cpSync(join(launcherDir, "bin"), join(main, "bin"), { recursive: true });
  chmodSync(join(main, "bin", "figstudio-mcp.js"), 0o755);
  copyFileSync(join(launcherDir, "README.md"), join(main, "README.md"));
  copyFileSync(license, join(main, "LICENSE"));
  const pkg = { ...shared, version, optionalDependencies: {} };
  for (const key of Object.keys(PLATFORMS)) pkg.optionalDependencies[packageName(key)] = version;
  writeFileSync(join(main, "package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
  dirs.push(main);

  if (pack) {
    for (const dir of dirs) {
      execFileSync("npm", ["pack", "--pack-destination", out], { cwd: dir, stdio: ["ignore", "ignore", "inherit"] });
    }
  }
  return dirs;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const dirs = assemble(parse(process.argv.slice(2)));
    for (const dir of dirs) console.log(dir);
  } catch (error) {
    console.error(`npm-pack: ${error.message}`);
    process.exit(2);
  }
}
