"""QA: figstudio cloud relay (#18) with two devices sharing one room.

Runs against a local `wrangler dev` of web/worker.mjs (real workerd + Durable Object).
Each fake executor proves the same executor secret (like a copied room link), answers
every rpc with its own name, and lets the script decide when it says `ready`.

Expectations encode the behaviour of the relay at 786d11c (see
docs/MULTI-DEVICE-SESSIONS-PLAN.md section 1.2); update them when routing changes.

    mkdir -p web/dist && printf '<!doctype html>' > web/dist/index.html
    wrangler dev --local --port 8799 --ip 127.0.0.1 --compatibility-date 2026-04-22
    pip install websockets   # 15.x
    BASE=http://127.0.0.1:8799 python3 docs/qa/relay_multi_device_qa.py
"""
import asyncio, hashlib, json, os, secrets, sys, time
import urllib.request, urllib.error
import websockets

BASE = os.environ.get("BASE", "http://127.0.0.1:8799")
WS = BASE.replace("http", "ws", 1)
EXEC_PROTOCOL = "figstudio-exec-v1"

def derive_room(ex_hex: str) -> str:
    pre = b"figstudio/room/v1" + b"\x00" + bytes.fromhex(ex_hex)
    return "rm_" + hashlib.sha256(pre).hexdigest()

def post(room, body, timeout=40):
    req = urllib.request.Request(f"{BASE}/mcp/{room}", data=json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"}, method="POST")
    t0 = time.monotonic()
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            code, text = r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        code, text = e.code, e.read().decode()
    return code, (json.loads(text) if text else None), round(time.monotonic() - t0, 2)

async def apost(room, body, timeout=40):
    return await asyncio.to_thread(post, room, body, timeout)

class Executor:
    def __init__(self, name, ex_hex, room, auto_reply=True):
        self.name, self.ex, self.room, self.auto_reply = name, ex_hex, room, auto_reply
        self.ws, self.seen, self.task, self.held = None, [], None, []

    async def connect(self):
        self.ws = await websockets.connect(
            f"{WS}/bridge/{self.room}", subprotocols=[EXEC_PROTOCOL, self.ex],
            origin=BASE, open_timeout=10)
        self.task = asyncio.create_task(self.loop())
        return self

    async def ready(self):
        await self.ws.send(json.dumps({"type": "ready", "protocol": 1}))
        await asyncio.sleep(0.15)

    async def loop(self):
        try:
            async for msg in self.ws:
                if msg == "pong":
                    continue
                frame = json.loads(msg)
                if frame.get("type") != "rpc":
                    continue
                self.seen.append(frame["body"].get("method"))
                if self.auto_reply:
                    await self.answer(frame)
                else:
                    self.held.append(frame)
        except websockets.ConnectionClosed:
            pass

    async def answer(self, frame):
        body = frame["body"]
        await self.ws.send(json.dumps({"type": "rpc_reply", "id": frame["id"], "status": 200,
            "body": {"jsonrpc": "2.0", "id": body["id"], "result": {"answered_by": self.name}}}))

    async def close(self):
        await self.ws.close()
        await asyncio.sleep(0.3)

def who(resp):
    code, body, secs = resp
    if body and "result" in body:
        return body["result"].get("answered_by")
    return f"HTTP {code} {body and body.get('error', {}).get('message')} ({secs}s)"

results = []
def check(name, got, expect, note=""):
    ok = got == expect
    results.append((ok, name, got, expect, note))
    print(f"{'PASS' if ok else 'FAIL'}  {name}: got={got!r} expect={expect!r} {note}")

async def main():
    ex = secrets.token_hex(32)
    room = derive_room(ex)
    rpc = lambda i, m="tools/list": {"jsonrpc": "2.0", "id": i, "method": m}

    code, body, _ = await apost(room, rpc(1))
    check("S0 no tab -> 503", (code, body["error"]["message"]), (503, "No browser tab connected"))

    office = await (Executor("office", ex, room)).connect(); await office.ready()
    check("S1 only office ready", who(await apost(room, rpc(2))), "office")

    home = await (Executor("home", ex, room)).connect()
    check("S2a home connected but not ready: office still answers",
          who(await apost(room, rpc(3))), "office")
    await home.ready()
    check("S2b home says ready -> calls silently move to home",
          who(await apost(room, rpc(4))), "home", "(caller is not told)")

    await office.ready()  # what the app does on any tab/selection change
    check("S3 office re-sends ready (selection change) -> calls jump back to office",
          who(await apost(room, rpc(5))), "office")

    # MCP session continuity: initialize on one device, tools/call on another.
    init_by = who(await apost(room, rpc(6, "initialize")))
    await home.ready()
    call_by = who(await apost(room, rpc(7, "tools/call")))
    check("S4 initialize and next tools/call land on different devices",
          (init_by, call_by), ("office", "home"), "(one MCP session spans two machines)")

    # Network blip on office: reconnect + ready steals the role back without user action.
    await office.close()
    check("S5 active office gone -> falls back to home", who(await apost(room, rpc(8))), "home")
    office2 = await (Executor("office", ex, room)).connect()
    check("S6a office reconnected, not ready yet -> home", who(await apost(room, rpc(9))), "home")
    await office2.ready()
    check("S6b office reconnect+ready steals calls from the user at home",
          who(await apost(room, rpc(10))), "office", "(idle machine wins after a blip)")

    # In-flight call stays bound to the socket that received it.
    slow = await (Executor("slow-home", ex, room, auto_reply=False)).connect(); await slow.ready()
    pending = asyncio.create_task(apost(room, rpc(11)))
    await asyncio.sleep(0.5)
    await office2.ready()
    await slow.answer(slow.held.pop())
    check("S7 in-flight call answered by the socket it was sent to",
          who(await pending), "slow-home")

    # In-flight call when that tab closes: fails fast, not after 30 s.
    await slow.ready()
    pending = asyncio.create_task(apost(room, rpc(12)))
    await asyncio.sleep(0.5)
    await slow.close()
    code, body, secs = await pending
    check("S8 tab closes with call in flight -> fast 503",
          (code, body["error"]["message"], secs < 5), (503, "Browser tab disconnected", True),
          f"({secs}s)")

    # 16 open sockets per room (all devices share the cap).
    extra = []
    for i in range(16):
        try:
            extra.append(await (Executor(f"x{i}", ex, room)).connect())
        except Exception as e:
            check("S9 16-socket cap reached", len(extra) + 2, 16, f"refused: {type(e).__name__}")
            break
    for e in extra:
        await e.close()
    await office2.close(); await home.close()

    code, body, _ = await apost(room, rpc(13))
    check("S10 all gone -> 503", (code, body["error"]["message"]), (503, "No browser tab connected"))
    fails = [r for r in results if not r[0]]
    print(f"\n{len(results) - len(fails)}/{len(results)} expectations matched")
    return 1 if fails else 0

sys.exit(asyncio.run(main()))
