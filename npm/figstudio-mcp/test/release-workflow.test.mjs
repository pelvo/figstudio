// Checks on .github/workflows/release-please.yml that a release cannot exercise until it
// runs. Kept with the launcher tests because this is the Node test slice of scripts/check.sh.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");
const workflow = (name) => readFileSync(join(root, ".github", "workflows", name), "utf8");

/** The jobs of a workflow as { name: text }, split on the two-space job keys under `jobs:`. */
function jobs(text) {
  const body = text.slice(text.search(/^jobs:\s*$/m));
  const out = {};
  let name;
  for (const line of body.split("\n").slice(1)) {
    const key = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line);
    if (key) {
      name = key[1];
      out[name] = "";
    } else if (name) {
      out[name] += `${line}\n`;
    }
  }
  return out;
}

test("the release passes the cloudflare environment's secrets to web-deploy", () => {
  // A reusable workflow sees only the secrets its caller passes, even for a job that names
  // an environment: without them CLOUDFLARE_API_TOKEN is empty and a release's web deploy
  // fails, although a manual web-deploy run with the same environment works.
  const deploy = jobs(workflow("web-deploy.yml")).deploy;
  assert.match(deploy, /^ {4}environment:\s*\n\s+name: cloudflare$/m, "web-deploy's deploy job uses the cloudflare environment");
  assert.match(deploy, /secrets\.CLOUDFLARE_API_TOKEN/, "web-deploy reads CLOUDFLARE_API_TOKEN");
  const callers = Object.entries(jobs(workflow("release-please.yml"))).filter(([, text]) =>
    /^ {4}uses: \.\/\.github\/workflows\/web-deploy\.yml\s*$/m.test(text),
  );
  assert.ok(callers.length > 0, "release-please.yml calls web-deploy.yml");
  for (const [name, text] of callers) {
    assert.match(text, /^ {4}secrets: inherit\s*$/m, `release-please.yml job ${name} passes its secrets to web-deploy`);
  }
});
