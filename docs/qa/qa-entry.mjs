// QA-only Worker entry (docs/MULTI-DEVICE-SESSIONS-PLAN.md section 1.3). The relay
// under test, unchanged, plus one log line per Durable Object construction, so
// `wrangler tail` shows when the object woke from hibernation or was re-created.
// Deploy it under a throwaway name, never over the production Worker:
//   wrangler deploy docs/qa/qa-entry.mjs --name figstudio-relay-qa-tmp
import worker, { McpRoom as Base } from "../../web/worker.mjs";

export default worker;

export class McpRoom extends Base {
  constructor(ctx, env) {
    console.log(JSON.stringify({ qa: "McpRoom constructed",
      sockets: ctx.getWebSockets("tab").length, at: Date.now() }));
    super(ctx, env);
  }
}
