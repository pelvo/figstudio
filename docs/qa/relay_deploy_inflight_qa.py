"""QA on real Cloudflare: redeploy while a call is in flight (the object is awake).

A slow executor receives an RPC and holds it. The script then waits for DEPLOY_FLAG to
appear (the operator redeploys meanwhile), answers the held RPC, and reports what the
caller got and whether the executor socket survived.
"""
import asyncio, os, secrets, time, sys
sys.path.insert(0, os.path.dirname(__file__))
from relay_multi_device_qa import Executor, derive_room, apost, who  # noqa: E402

FLAG = os.environ.get("DEPLOY_FLAG", "deployed.flag")

async def main():
    ex = secrets.token_hex(32)
    room = derive_room(ex)
    slow = await Executor("slow", ex, room, auto_reply=False).connect()
    await slow.ready()
    t0 = time.time()
    pending = asyncio.create_task(apost(room, {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
    while not slow.held:
        await asyncio.sleep(0.1)
    print("rpc held by executor; redeploy now", flush=True)
    while not os.path.exists(FLAG) and not pending.done():
        await asyncio.sleep(0.2)
    print(f"+{time.time() - t0:.1f}s deploy flag seen; socket={slow.ws.state.name}", flush=True)
    if slow.ws.state.name == "OPEN" and slow.held:
        try:
            await slow.answer(slow.held.pop())
        except Exception as e:
            print("answer failed:", type(e).__name__, flush=True)
    print(f"+{time.time() - t0:.1f}s caller got:", who(await pending), flush=True)
    await asyncio.sleep(2)
    print("socket after:", slow.ws.state.name, "close_code:", slow.ws.close_code, flush=True)
    if slow.ws.state.name == "OPEN":
        await slow.ready()
        slow.auto_reply = True
        print("next call:", who(await apost(room, {"jsonrpc": "2.0", "id": 2, "method": "ping"})), flush=True)
        await slow.close()

asyncio.run(main())
