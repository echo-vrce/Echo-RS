r"""Record an Echo VR *client* session end to end (no admin / pcap needed).

    python tools/record_client.py <out-dir> <game-dir> [echovr args...]

1. Backs up `<game-dir>\_local\config.json` and points its websocket services
   (login, matching, config, transaction, social, serverdb) at this proxy, which
   forwards each to the original host and logs every message both ways.
2. Rewrites LobbySessionSuccessV5 so the game server's endpoint is a local UDP
   relay: every game datagram is logged, then forwarded to the real server.
   The session keys in that message (logged too) decrypt the UDP log later.
3. Polls the game's HTTP API (`-httpport`, default 6721) `/session` into api.jsonl.
4. Starts `bin\win10\echovr.exe <args>`; when it exits (or on Ctrl+C) the
   original config.json is put back.

Output (out-dir):
  ws.jsonl   {"t", "dir": "c2s"|"s2c", "svc", "hex"|"text"}
  udp.bin    records <d t><B dir 0=c2s 1=s2c><4s server ip><H server port><I len><data>
  api.jsonl  {"t", "status", "body"} for each /session poll that changed
  events.log notes (session endpoints, keys file)
  sessions.jsonl one line per LobbySessionSuccessV5 (endpoint + keys, hex)

Query strings (credentials) are never written to the logs.
"""
import asyncio, json, os, shutil, signal, socket, struct, subprocess, sys, time, urllib.request
from urllib.parse import urlsplit

import websockets

WS_PORT = int(os.environ.get("REC_WS_PORT", "7790"))
UDP_PORT = int(os.environ.get("REC_UDP_PORT", "6830"))
MARKER = bytes.fromhex("f640bb78a2e78cbb")
SESSION_SUCCESS_V5 = 0x6D4DE3650EE3110F
SERVICES = ["loginservice_host", "matchingservice_host", "configservice_host",
            "transactionservice_host", "socialservice_host", "serverdb_host"]

OUT_DIR, GAME_DIR, GAME_ARGS = sys.argv[1], sys.argv[2], sys.argv[3:]
os.makedirs(OUT_DIR, exist_ok=True)
ws_log = open(os.path.join(OUT_DIR, "ws.jsonl"), "a", buffering=1)
udp_log = open(os.path.join(OUT_DIR, "udp.bin"), "ab", buffering=0)
api_log = open(os.path.join(OUT_DIR, "api.jsonl"), "a", buffering=1)
ev_log = open(os.path.join(OUT_DIR, "events.log"), "a", buffering=1)
sess_log = open(os.path.join(OUT_DIR, "sessions.jsonl"), "a", buffering=1)

CONFIG = os.path.join(GAME_DIR, "_local", "config.json")
BACKUP = CONFIG + ".bak-record-client"
upstreams = {}  # service key -> original url (with query)
server = [None]  # real (ip, port) of the current game server
counts = [0, 0]


def note(s):
    line = f"{time.strftime('%H:%M:%S')} {s}"
    print(line, flush=True)
    ev_log.write(line + "\n")


def patch_config():
    if not os.path.exists(BACKUP):
        shutil.copyfile(CONFIG, BACKUP)
    cfg = json.load(open(BACKUP))
    for k in SERVICES:
        url = cfg.get(k)
        if isinstance(url, str) and url.startswith(("ws://", "wss://")):
            upstreams[k] = url
            q = urlsplit(url).query
            cfg[k] = f"ws://127.0.0.1:{WS_PORT}/{k}" + (f"?{q}" if q else "")
    json.dump(cfg, open(CONFIG, "w"), indent=2)
    note(f"config patched: {', '.join(f'{k}->{urlsplit(u).netloc}' for k, u in upstreams.items())}")


def restore_config():
    if os.path.exists(BACKUP):
        shutil.copyfile(BACKUP, CONFIG)
        os.remove(BACKUP)
        note("config.json restored")


def rewrite_session(data: bytes) -> bytes:
    buf = bytearray(data)
    i = 0
    while i + 24 <= len(buf) and buf[i:i + 8] == MARKER:
        sym, ln = struct.unpack_from("<QQ", buf, i + 8)
        p = i + 24
        if sym == SESSION_SUCCESS_V5 and ln >= 50:
            internal = socket.inet_ntoa(bytes(buf[p + 40:p + 44]))
            external = socket.inet_ntoa(bytes(buf[p + 44:p + 48]))
            port = struct.unpack_from(">H", buf, p + 48)[0]
            sess_log.write(json.dumps({"t": time.time(), "internal": internal, "external": external,
                                       "port": port, "payload": bytes(buf[p:p + ln]).hex()}) + "\n")
            server[0] = (external, port)
            buf[p + 40:p + 44] = socket.inet_aton("127.0.0.1")
            buf[p + 44:p + 48] = socket.inet_aton("127.0.0.1")
            struct.pack_into(">H", buf, p + 48, UDP_PORT)
            note(f"session: game server {external}:{port} (internal {internal}); relaying via :{UDP_PORT}")
        i = p + ln
    return bytes(buf)


async def ws_handler(client):
    path = client.request.path if hasattr(client, "request") else client.path
    svc = urlsplit(path).path.strip("/").split("/")[0]
    up_url = upstreams.get(svc)
    if not up_url:
        note(f"unknown service path {urlsplit(path).path}")
        return
    note(f"client connected: {svc} -> {urlsplit(up_url).netloc}")
    try:
        async with websockets.connect(up_url, max_size=None, ping_interval=None) as up:
            async def pipe(src, dst, d):
                async for m in src:
                    if d == "s2c" and isinstance(m, bytes):
                        m = rewrite_session(m)
                    rec = {"t": time.time(), "dir": d, "svc": svc}
                    rec["hex" if isinstance(m, bytes) else "text"] = m.hex() if isinstance(m, bytes) else m
                    ws_log.write(json.dumps(rec) + "\n")
                    await dst.send(m)
            tasks = [asyncio.create_task(pipe(client, up, "c2s")), asyncio.create_task(pipe(up, client, "s2c"))]
            _, pending = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for t in pending:
                t.cancel()
    except Exception as e:
        note(f"{svc}: {type(e).__name__}: {e}")
    note(f"closed: {svc}")


def log_udp(direction, addr, data):
    udp_log.write(struct.pack("<dB4sHI", time.time(), direction, socket.inet_aton(addr[0]), addr[1], len(data)) + data)
    counts[direction] += 1


class Up(asyncio.DatagramProtocol):
    def __init__(self, front, client, target):
        self.front, self.client, self.target = front, client, target

    def datagram_received(self, data, addr):
        log_udp(1, self.target, data)
        self.front.transport.sendto(data, self.client)


class Front(asyncio.DatagramProtocol):
    def __init__(self):
        self.peers = {}

    def connection_made(self, transport):
        self.transport = transport

    def datagram_received(self, data, addr):
        target = server[0]
        if target is None:
            return
        log_udp(0, target, data)
        key = (addr, target)
        peer = self.peers.get(key)
        if peer is None:
            loop = asyncio.get_running_loop()
            peer = loop.create_task(loop.create_datagram_endpoint(lambda: Up(self, addr, target), remote_addr=target))
            self.peers[key] = peer
            note(f"udp: client {addr[1]} <-> {target[0]}:{target[1]}")
        if peer.done():
            peer.result()[0].sendto(data)
        else:
            peer.add_done_callback(lambda f, d=data: f.result()[0].sendto(d))


def http_port():
    for i, a in enumerate(GAME_ARGS):
        if a == "-httpport" and i + 1 < len(GAME_ARGS):
            return int(GAME_ARGS[i + 1])
    return 6721


async def poll_api():
    url, last = f"http://127.0.0.1:{http_port()}/session", None
    def get():
        try:
            with urllib.request.urlopen(url, timeout=1) as r:
                return r.status, r.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as e:
            return e.code, e.read().decode("utf-8", "replace")
        except Exception:
            return None, None
    while True:
        status, body = await asyncio.to_thread(get)
        if status is not None and (status, body) != last:
            api_log.write(json.dumps({"t": time.time(), "status": status, "body": body}) + "\n")
            last = (status, body)
        await asyncio.sleep(0.2)


async def stats():
    while True:
        await asyncio.sleep(15)
        print(f"udp c2s={counts[0]} s2c={counts[1]}", flush=True)


async def main():
    patch_config()
    loop = asyncio.get_running_loop()
    await loop.create_datagram_endpoint(Front, local_addr=("0.0.0.0", UDP_PORT))
    async with websockets.serve(ws_handler, "127.0.0.1", WS_PORT, max_size=None, ping_interval=None):
        exe = os.path.join(GAME_DIR, "bin", "win10", "echovr.exe")
        game = subprocess.Popen([exe, *GAME_ARGS], cwd=os.path.dirname(exe))
        note(f"recording to {OUT_DIR}; game pid {game.pid}; args {' '.join(GAME_ARGS)}")
        tasks = [asyncio.create_task(poll_api()), asyncio.create_task(stats())]
        while game.poll() is None:
            await asyncio.sleep(1)
        note(f"game exited ({game.returncode})")
        for t in tasks:
            t.cancel()


try:
    asyncio.run(main())
finally:
    restore_config()
    note(f"done: udp c2s={counts[0]} s2c={counts[1]}")
