// Standalone tests for web/executor-auth.mjs — no Worker import, no installs.
// The derivation vectors are the shared contract with
// apps/figstudio/src/mcp_pair_tests.rs: same domain, same bytes, same rooms.
import assert from "node:assert/strict";
import { test } from "node:test";
import {
  ROOM_DOMAIN, EXEC_PROTOCOL, deriveRoom, checkBridgeUpgrade,
} from "../executor-auth.mjs";

if (!globalThis.crypto) {
  const { webcrypto } = await import("node:crypto");
  globalThis.crypto = webcrypto;
}

const ZEROS = "ex_".padEnd(67, "0");
const SEQUENCE = "ex_000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const ORIGIN = "https://relay.example";

test("the derivation matches the shared vector", async () => {
  assert.equal(await deriveRoom(ZEROS.slice(3)),
    "rm_f387ed88cf9a2c064c95e23d5b96cb76e2c9af734b41febc643dd57b90fb7525");
  assert.equal(await deriveRoom(SEQUENCE.slice(3)),
    "rm_9851486d85fc5fe5c0d9082ab07c4d165e75120278631e4e221905f821b180c1");
  // Only lowercase hex of exactly 32 bytes derives. (Upper-casing the all-zeros
  // hex is a no-op, so the case check uses a hex string with letters.)
  for (const bad of ["", "abc", SEQUENCE.slice(3).toUpperCase(), ZEROS.slice(3).slice(0, 63), "0x00", null]) {
    assert.equal(await deriveRoom(bad), null, String(bad));
  }
  assert.equal(ROOM_DOMAIN, "figstudio/room/v1");
  assert.equal(EXEC_PROTOCOL, "figstudio-exec-v1");
});

const upgrade = (path, overrides = {}) => new Request(`https://relay.example${path}`, {
  method: "GET",
  headers: {
    Upgrade: "websocket",
    Origin: ORIGIN,
    "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${SEQUENCE.slice(3)}`,
    ...overrides,
  },
});

test("a matching bridge upgrade proves the executor before the DO", async () => {
  const room = await deriveRoom(SEQUENCE.slice(3));
  const proof = await checkBridgeUpgrade(upgrade(`/bridge/${room}`));
  assert.deepEqual(proof, { ok: true, room, executorHex: SEQUENCE.slice(3) });
});

test("every other bridge upgrade is refused without echoing the secret", async () => {
  const room = await deriveRoom(SEQUENCE.slice(3));
  const other = await deriveRoom(ZEROS.slice(3));
  const attempts = [
    [upgrade("/bridge/not-a-room"), 404],
    [upgrade("/mcp/" + room), 404],
    [upgrade(`/bridge/${room}`, { Upgrade: "h2c" }), 426],
    [new Request(`https://relay.example/bridge/${room}`, { method: "GET", headers: { Origin: ORIGIN } }), 426],
    [upgrade(`/bridge/${room}`, { Origin: "https://evil.example" }), 403],
    [upgrade(`/bridge/${room}`, { Origin: "" }), 403],
    [upgrade(`/bridge/${other}`), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": SEQUENCE.slice(3) }), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": `${SEQUENCE.slice(3)}, ${EXEC_PROTOCOL}` }), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${SEQUENCE.slice(3)}, extra` }), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, ${SEQUENCE.slice(3).toUpperCase()}` }), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": "" }), 403],
    [upgrade(`/bridge/${room}`, { "Sec-WebSocket-Protocol": `${EXEC_PROTOCOL}, short` }), 403],
  ];
  for (const [request, status] of attempts) {
    const proof = await checkBridgeUpgrade(request);
    assert.equal(proof.ok, false, request.url);
    assert.equal(proof.status, status, `${request.url} ${JSON.stringify(proof.message)}`);
    assert.equal(typeof proof.message, "string");
    // The refusal names the rule, never the offered values.
    assert.ok(!JSON.stringify(proof).includes(SEQUENCE.slice(3)), JSON.stringify(proof));
    assert.ok(!JSON.stringify(proof).includes(SEQUENCE), JSON.stringify(proof));
  }
});
