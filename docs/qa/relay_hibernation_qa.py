"""QA on real Cloudflare: does routing survive Durable Object hibernation?

Two executors in one room say `ready` (office, then home), everything goes idle for
IDLE seconds (only WebSocket protocol pings, which the runtime answers without waking
the object), then one RPC is sent. Run `wrangler tail` alongside to see whether the
object was constructed again (= woke from hibernation) before answering.
"""
import asyncio, os, secrets, sys, time
sys.path.insert(0, os.path.dirname(__file__))
from relay_multi_device_qa import Executor, derive_room, apost, who  # noqa: E402

IDLE = int(os.environ.get("IDLE", "75"))

async def main():
    ex = secrets.token_hex(32)
    room = derive_room(ex)
    rpc = lambda i: {"jsonrpc": "2.0", "id": i, "method": "tools/list"}
    office = await Executor("office", ex, room).connect(); await office.ready()
    home = await Executor("home", ex, room).connect(); await home.ready()
    print("before idle:", who(await apost(room, rpc(1))), flush=True)
    t0 = time.time()
    print(f"idle {IDLE}s from {t0:.0f}", flush=True)
    await asyncio.sleep(IDLE)
    print("after idle:", who(await apost(room, rpc(2))), flush=True)
    await office.ready()
    print("office ready after wake:", who(await apost(room, rpc(3))), flush=True)
    for e in (office, home):
        print(e.name, "socket open:", e.ws.state.name, flush=True)
    if os.environ.get("HOLD"):
        print(f"holding sockets {os.environ['HOLD']}s (redeploy now)", flush=True)
        t = time.time()
        while time.time() - t < int(os.environ["HOLD"]):
            await asyncio.sleep(1)
            states = {e.name: e.ws.state.name for e in (office, home)}
            if any(s != "OPEN" for s in states.values()):
                print(f"+{time.time() - t:.0f}s states={states} close_codes=",
                      {e.name: e.ws.close_code for e in (office, home)}, flush=True)
                break
        states = {e.name: e.ws.state.name for e in (office, home)}
        print(f"after hold: states={states}", flush=True)
        print("call after redeploy:", who(await apost(room, rpc(4))), flush=True)
    await office.close(); await home.close()

asyncio.run(main())
