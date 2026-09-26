#!/usr/bin/env python3
"""Raw-bytes proxy in front of the remote socket, for when a frame the server
believes it sent never reaches the client: connect a client to this port and
it prints the first bytes the server writes, unmasked and unparsed.

usage: python3 scripts/probe_remote_wire.py [port] [upstream]
       node scripts/dump-remote-frame.mjs   # against 127.0.0.1:8899
"""
import asyncio
import sys
import time

LISTEN = int(sys.argv[1]) if len(sys.argv) > 1 else 8899
UPSTREAM = int(sys.argv[2]) if len(sys.argv) > 2 else 8765
DUMP = 4000


START = time.monotonic()
CONN = [0]


async def pipe(reader, writer, tag, cid):
    total = 0
    try:
        while total < DUMP:
            chunk = await reader.read(4096)
            if not chunk:
                print(f"[{time.monotonic() - START:6.2f}s c{cid} {tag}] EOF")
                break
            writer.write(chunk)
            await writer.drain()
            total += len(chunk)
            head = chunk[:24]
            print(
                f"[{time.monotonic() - START:6.2f}s c{cid} {tag}] +{len(chunk)} "
                + (head.decode("utf8", "replace") if b"HTTP" in chunk[:8] else head.hex())
            )
    except (ConnectionResetError, BrokenPipeError) as exc:
        print(f"[{time.monotonic() - START:6.2f}s c{cid} {tag}] putus: {exc}")
    finally:
        writer.close()


async def main():
    async def handler(r, w):
        CONN[0] += 1
        cid = CONN[0]
        ur, uw = await asyncio.open_connection("127.0.0.1", UPSTREAM)
        await asyncio.gather(pipe(r, uw, "client->server", cid), pipe(ur, w, "server->client", cid))

    srv = await asyncio.start_server(handler, "127.0.0.1", LISTEN)
    print(f"proxy 127.0.0.1:{LISTEN} -> 127.0.0.1:{UPSTREAM}")
    async with srv:
        await srv.serve_forever()


asyncio.run(main())
