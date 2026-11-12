"""Record a native Echo VR game server session end to end.

The native server's serverdb_host points at this proxy (WS_PORT). Every
websocket message both ways is logged, including the session keys EchoRelay
hands the server. The server's registration is rewritten to advertise
UDP_PORT, so clients send their game traffic here and every UDP datagram is
logged before being relayed to the server's real port.

Output (in OUT_DIR):
  ws.jsonl   {"t": unix, "dir": "s2r"|"r2s", "path": ..., "hex": ...}
  udp.bin    records: <d t><B dir 0=c2s 1=s2c><4s client ip><H client port><I len><data>
  events.log human-readable notes
"""
import asyncio, json, os, struct, sys, time, socket
import websockets

UPSTREAM = "ws://127.0.0.1:777"
WS_PORT = 7781
UDP_PORT = 6800
MARKER = bytes.fromhex("f640bb78a2e78cbb")
REG = 0x7777777777777777

OUT_DIR = sys.argv[1]
os.makedirs(OUT_DIR, exist_ok=True)
ws_log = open(os.path.join(OUT_DIR, "ws.jsonl"), "a", buffering=1)
udp_log = open(os.path.join(OUT_DIR, "udp.bin"), "ab", buffering=0)
ev_log = open(os.path.join(OUT_DIR, "events.log"), "a", buffering=1)

native_port = None
udp_count = [0, 0]


def note(s):
    line = f"{time.strftime('%H:%M:%S')} {s}"
    print(line, flush=True)
    ev_log.write(line + "\n")


def rewrite_registration(data: bytes) -> bytes:
    """Swap the advertised UDP port in a ServerDB registration for ours."""
    global native_port
    buf = bytearray(data)
    i = 0
    while i + 24 <= len(buf):
        if buf[i:i + 8] != MARKER:
            break
        sym, ln = struct.unpack_from("<QQ", buf, i + 8)
        p = i + 24
        if sym == REG and ln >= 14:
            port = struct.unpack_from("<H", buf, p + 12)[0]
            native_port = port
            struct.pack_into("<H", buf, p + 12, UDP_PORT)
            note(f"registration: native UDP port {port}, advertising {UDP_PORT}")
        i = p + ln
    return bytes(buf)


async def ws_handler(client):
    path = client.request.path if hasattr(client, "request") else client.path
    note(f"server connected to proxy path={path}")
    async with websockets.connect(UPSTREAM + path, max_size=None) as up:
        async def s2r():
            async for m in client:
                if isinstance(m, bytes):
                    m = rewrite_registration(m)
                ws_log.write(json.dumps({"t": time.time(), "dir": "s2r", "path": path,
                                         "hex": m.hex() if isinstance(m, bytes) else None,
                                         "text": m if isinstance(m, str) else None}) + "\n")
                await up.send(m)

        async def r2s():
            async for m in up:
                ws_log.write(json.dumps({"t": time.time(), "dir": "r2s", "path": path,
                                         "hex": m.hex() if isinstance(m, bytes) else None,
                                         "text": m if isinstance(m, str) else None}) + "\n")
                await client.send(m)

        done, pending = await asyncio.wait([asyncio.create_task(s2r()), asyncio.create_task(r2s())],
                                           return_when=asyncio.FIRST_COMPLETED)
        for t in pending:
            t.cancel()
    note(f"server proxy connection closed path={path}")


def log_udp(direction, addr, data):
    ip = socket.inet_aton(addr[0])
    udp_log.write(struct.pack("<dB4sHI", time.time(), direction, ip, addr[1], len(data)) + data)
    udp_count[direction] += 1


class Upstream(asyncio.DatagramProtocol):
    def __init__(self, front, client_addr):
        self.front, self.client_addr = front, client_addr

    def datagram_received(self, data, addr):
        log_udp(1, self.client_addr, data)
        self.front.transport.sendto(data, self.client_addr)


class Front(asyncio.DatagramProtocol):
    def __init__(self):
        self.peers = {}

    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, data, addr):
        log_udp(0, addr, data)
        if native_port is None:
            return
        peer = self.peers.get(addr)
        if peer is None:
            loop = asyncio.get_running_loop()
            peer = loop.create_task(loop.create_datagram_endpoint(
                lambda: Upstream(self, addr), remote_addr=("127.0.0.1", native_port)))
            self.peers[addr] = peer
            note(f"new UDP peer {addr[0]}:{addr[1]}")
            peer.add_done_callback(lambda f, d=data: f.result()[0].sendto(d))
            return
        if peer.done():
            peer.result()[0].sendto(data)
        else:
            peer.add_done_callback(lambda f, d=data: f.result()[0].sendto(d))


async def stats():
    while True:
        await asyncio.sleep(10)
        print(f"udp c2s={udp_count[0]} s2c={udp_count[1]}", flush=True)


async def main():
    loop = asyncio.get_running_loop()
    await loop.create_datagram_endpoint(Front, local_addr=("0.0.0.0", UDP_PORT))
    note(f"recording to {OUT_DIR}; ws proxy :{WS_PORT} -> {UPSTREAM}, udp relay :{UDP_PORT}")
    async with websockets.serve(ws_handler, "127.0.0.1", WS_PORT, max_size=None):
        await stats()


asyncio.run(main())
