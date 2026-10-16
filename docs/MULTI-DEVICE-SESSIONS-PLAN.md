# Multi-Device Sessions for the Cloud Relay — QA Report and Plan

Status: plan only. Nothing here is implemented. Base: `origin/main` at `786d11c`
(cloud relay from #18).

The relay works for one browser. It breaks down when the same person uses more than one
machine: the MCP URL changes per machine, and when two machines share a room the caller is
silently switched between them. This document records a QA pass over the merged relay,
then proposes a session model in which each executor reports who it is and the caller
picks one, instead of "the newest `ready` wins".

---

## 1. QA of the merged relay

### 1.1 Gates

| Gate | Result |
|---|---|
| `node --test web/worker/` | 33 / 33 pass |
| `cargo +1.99.0 test --release -p figstudio mcp_` | 59 / 59 pass |
| Local workerd run (1.2) | 13 / 13 scenarios behave as the code says |
| Real Cloudflare run (1.3) | 13 / 13 identical, plus hibernation and redeploy checks |

### 1.2 Live run against workerd

`wrangler dev --local` (wrangler 4.83.0) served `web/worker.mjs` with a real `McpRoom`
Durable Object and real WebSockets. The local runtime only supports compatibility dates up
to `2026-04-22`, so the run passed `--compatibility-date 2026-04-22`; the Worker uses no
API newer than that. `web/dist` was a one-line placeholder (the relay does not need the
app).

Two fake executors proved **the same executor secret** — what happens when a room link is
copied to a second machine. Each one answered every RPC with its own name, so the caller
can see who answered. The script is `docs/qa/relay_multi_device_qa.py`.

| # | Scenario | Observed |
|---|---|---|
| S0 | No executor connected | `503 No browser tab connected` |
| S1 | Office connects and says `ready` | office answers |
| S2a | Home connects, has not said `ready` | office still answers |
| S2b | Home says `ready` | **home answers; the caller is not told** |
| S3 | Office re-sends `ready` (the app does this on any tab or selection change) | **calls jump back to office** |
| S4 | `initialize` then `tools/call` with a `ready` from home in between | **`initialize` on office, `tools/call` on home: one MCP session spans two machines** |
| S5 | Active office socket closes | falls back to home (good) |
| S6a | Office reconnects, no `ready` yet | home keeps answering (good) |
| S6b | Office says `ready` after the reconnect | **office takes the calls back from the person at home** |
| S7 | Another executor says `ready` while a call is in flight | the in-flight call is answered by the socket it was sent to (good) |
| S8 | Executor closes with a call in flight | `503 Browser tab disconnected` after 0.5 s, not 30 s (good) |
| S9 | Socket cap | 16 open sockets per room, shared by all machines |
| S10 | Everything closed | `503 No browser tab connected` |

The app reconnects once on its own after a proven connection drops
(`apps/figstudio/src/mcp_web.rs:251`) and sends `ready` on connect, so S6b happens without
the user touching the idle machine: a Wi-Fi blip at the office is enough. (A Worker
deploy did **not** trigger this in the real-Cloudflare run below: the hibernated sockets
stayed open across deploys.)

### 1.3 Real Cloudflare run

A throwaway Worker (`figstudio-relay-qa-tmp` on `workers.dev`, same code as `786d11c`)
was deployed for this QA and deleted afterwards (the URL then returned 404). The
production origin and its Access application were not touched. A QA-only entry wrapped
`McpRoom` to log one line per construction, read with `wrangler tail`, so the run could
see when the object was re-created.

| # | Scenario | Observed |
|---|---|---|
| C1 | S0–S10 above | 13 / 13 identical to workerd (S8: 0.52 s) |
| C2 | Two executors `ready`, idle 75 s, then a call | object re-constructed with 2 sockets restored; the call went to the newest `ready` (home), as before the idle; a later office `ready` took over normally |
| C3 | Same with 15 s idle | re-constructed after the idle as well; routing unchanged |
| C4 | Redeploy (variable-only change) while the object hibernates | both sockets stayed open for 150 s; the next event constructed the object on the **new** version with both sockets |
| C5 | Redeploy with a code change while the object hibernates | same as C4; the call after the deploy went to the newest `ready` |
| C6 | Redeploy while a call is in flight (object awake) | the object kept running the **old** version, the held call completed normally, the socket stayed open, and the next call was still served by the old version |

Reproduce with `docs/qa/qa-entry.mjs` (deploy command in its header),
`docs/qa/relay_hibernation_qa.py` (C2–C5, `IDLE`/`HOLD` seconds) and
`docs/qa/relay_deploy_inflight_qa.py` (C6), then delete the Worker.

Observations, not guarantees: one run, one location (NRT), wrangler 4.83.0. Cloudflare's
WebSocket guide says code updates disconnect all WebSockets; with the Hibernation API
this run saw no disconnect on deploy, so the plan does not rely on either behaviour.

The default Python `urllib` User-Agent got Cloudflare error 1010 on `workers.dev`; the QA
script now sends its own User-Agent. MCP clients with unusual User-Agents may meet the
same account-level bot rules.

### 1.4 Findings

- **F1 — silent target switch (S2b, S3).** `activeTab()` (`web/worker.mjs:91`) picks the
  socket with the highest `order`, and every `ready` bumps it (`web/worker.mjs:222`).
  The caller cannot tell which executor answered: the reply carries no identity.
- **F2 — an idle machine steals the session after a network blip (S6b).**
- **F3 — one MCP session spans executors (S4).** `initialize`, `tools/list` and
  `tools/call` may each land on a different machine with a different set of open files.
- **F4 — identity is the executor secret.** A second machine is either a different room
  (a new MCP URL to paste into the client) or a copy of the same secret. With a copy, one
  machine cannot be revoked; **Regenerate** disconnects all of them.
- **F5 — closing the browser loses the session.** This is the zero-upload design, not a
  bug, but the error (`No browser tab connected`) does not say what to do.
- **F6 — the 16-socket cap is per room**, so it is shared by every machine. Fine today;
  list it in the session output so it is visible.

### 1.5 Not verified

- Eviction (as opposed to hibernation) on real Cloudflare: it could not be forced. The
  Node test `an eviction drops in-flight calls` remains the only evidence.
- The production deployment: it sits behind Cloudflare Access, so an unauthenticated
  `POST /mcp/<room>` is redirected to the Access login (302). No service token was used;
  the real-Cloudflare run used a separate throwaway Worker.
- The browser app end to end (Playwright e2e needs a built app; not run here), Safari, and
  background-tab throttling.

---

## 2. Goals and non-goals

Goals

- One MCP URL per person, the same on every machine.
- The caller can list the executors in a room and choose one.
- No silent switch between machines. Ambiguity is an error that lists the candidates.
- Errors that tell the user what to do.

Non-goals

- Uploading `.fig` files to Cloudflare. Files stay on the machine that opened them.
- Durable Object storage in phase 1. Everything fits in socket attachments.

---

## 3. Design

### 3.1 Session identity

Each executor reports, in a new `ready` (protocol 2):

```json
{
  "type": "ready", "protocol": 2,
  "device": { "id": "<128-bit hex>", "label": "Office · Chrome" },
  "instance": "<random per page load>",
  "focused": true,
  "docs": [ { "tab": 0, "name": "landing.fig", "key": "<16 hex>", "pages": 12 } ]
}
```

- `device.id` lives in `localStorage["figstudio.mcp.device.v1"]`. It is not a secret.
- `device.label` defaults to platform + browser and is editable under **Agent access**.
- `instance` separates two browser tabs on the same device.
- `docs[].key` is the first 16 hex of SHA-256 over the file size and its first 4 MiB, so
  the same file open on two machines is recognisable without hashing 1.7 GB.
- The session id shown to callers is `s_` + the first 8 hex of
  SHA-256(`device.id` ‖ `instance`).
- The registry lives in socket attachments, which survive hibernation while the socket is
  open. Cloudflare caps a serialized attachment at 16,384 bytes
  (developers.cloudflare.com/durable-objects/best-practices/websockets, "Extended
  methods"), so the relay caps `docs` (at most 32 entries, names truncated to 96
  characters) to stay well under it.
- Protocol 1 `ready` stays accepted. Such a socket is an anonymous session on an unknown
  device.

### 3.2 Routing (replaces `activeTab()`)

The Durable Object answers two tools itself and never forwards them:

- `list_sessions` — every ready session: id, device label, docs, `focused`, seconds since
  its last `ready`, and how many of the 16 sockets are in use.
- `select_session { session }` — optional sticky choice (phase 1b).

The relay adds both tools to the `tools/list` reply it forwards from the tab, and answers
`tools/list` with just these two when no tab is connected.

For a forwarded `tools/call`:

1. If `arguments.session` is present, use that session (strip the argument before
   forwarding). If it is not connected, return an error that lists the live sessions.
2. Otherwise, if a sticky choice exists for this caller (phase 1b), use it.
3. Otherwise, if every ready session is on **one device**, use that device's focused or
   newest session. This is today's behaviour, kept because several browser tabs on one
   machine belong to the same person at the same desk.
4. Otherwise (sessions on more than one device) return JSON-RPC error `-32002`
   "Several figstudio sessions are connected; pass `session`", with the
   `list_sessions` payload in `error.data`. The client picks one and retries.

Session-independent methods (`initialize`, `ping`, `tools/list`, `prompts/*`) keep going to
the newest session; every executor of the same build answers them identically.

Every forwarded `tools/call` result gets
`_meta: { "figstudio/session": "s_…", "figstudio/device": "<label>" }`, so the caller can
always see who answered (fixes the "not told" part of F1).

Phase 1b, sticky choice: the relay issues an `Mcp-Session-Id` on `initialize` and keeps
`{mcp-session → figstudio session}` in memory. After hibernation or eviction the map is
gone and routing falls back to rules 3–4, which is safe: the worst case is one extra
ambiguity error.

### 3.3 Linking devices and revocation

- **Link another device** is the existing room link. The menu should say plainly that it
  carries executor authority.
- Revoking one device needs per-device credentials (each device proves its own secret,
  enrolled into the room by an existing device). That needs Durable Object storage for the
  enrolment list, so it is phase 4. Until then, revocation is Regenerate plus re-linking
  the devices you keep.
- Alternative for phase 4: derive the room from a Cloudflare Access identity
  (`Cf-Access-Authenticated-User-Email`). There is no linking step, but it ties the
  feature to Access.

### 3.4 When the browser is closed

- Better error: "No figstudio session is connected to this room. Open `<origin>` on a
  linked device and load the file."
- **Reopen last file** (Chromium): keep the `FileSystemFileHandle` from
  `showOpenFilePicker` in IndexedDB. On the next visit, one click plus the permission
  prompt reloads the file. Nothing is uploaded. Other browsers keep today's flow.
- **Native executor**: `figstudio-mcp --relay '<room link>'` joins the room as an
  executor labelled "<host> (native)" and serves files from local disk through
  `mcp-core`. An always-on machine then answers without a browser open. The native client
  must send the same `Origin` header the bridge check requires; the executor secret, not
  the Origin, is what authenticates it.

---

## 4. Phases and acceptance

| Phase | Scope | Done when |
|---|---|---|
| 1 | Worker: `ready` v2 parsing, attachment registry with the size cap, routing rules 1, 3, 4, `list_sessions`, `_meta`, the error with candidates | Node tests for each rule; the QA script, updated, shows S2b/S3/S4/S6b return `-32002` or stay on the explicitly chosen session; every new test fails under a mutation first |
| 1b | Sticky `select_session` via `Mcp-Session-Id` | Node test: the choice survives a `ready` from another device and is dropped safely on a simulated eviction |
| 1c | App: `device.id`, editable label, `ready` v2 with `docs` and `focused` | Rust wire tests; Chromium e2e with two browser contexts sharing one room link |
| 2 | Clearer errors, **Reopen last file** | e2e: reload page → reopen → `get_metadata` works without picking the file again |
| 3 | Native executor joins the relay | Integration test against `wrangler dev`: native session listed, answers a read-only tool |
| 4 | Per-device revocation or an Access-derived room | Decided by the owner; needs storage |

Gates for every phase: `FIGSTUDIO_TOOLCHAIN=1.99.0 scripts/check.sh --web` and
`node --test web/worker/`.

---

## 5. Open questions

1. Rule 3 keeps "newest wins" within one device. Is that acceptable, or should several
   tabs on one machine also require a choice?
2. Is phase 1b's in-memory sticky choice enough, or is a small amount of Durable Object
   storage acceptable to keep it across hibernation?
3. Should `focused` count when routing (rule 3), or stay display-only?
4. Phase 4: per-device secrets, or a room derived from the Access identity?
