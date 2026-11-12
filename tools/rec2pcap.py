"""Turn a record_native.py capture into a classic pcap.

UDP records become Ethernet/IPv4/UDP frames between the client's real address
and SERVER (port 6792). EchoRelay -> server websocket payloads become one TCP
flow carrying the raw SNS frames, so key scanners find SessionSuccess v5.
"""
import json, struct, sys, os

SERVER_IP = bytes([10, 0, 0, 1])
SERVER_PORT = 6792
RELAY_IP = bytes([10, 0, 0, 2])


def csum(b):
    if len(b) % 2:
        b += b"\0"
    s = sum(struct.unpack(f"!{len(b)//2}H", b))
    while s >> 16:
        s = (s & 0xFFFF) + (s >> 16)
    return ~s & 0xFFFF


def ip(src, dst, proto, payload, ident):
    h = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(payload), ident & 0xFFFF, 0, 64, proto, 0, src, dst)
    h = h[:10] + struct.pack("!H", csum(h)) + h[12:]
    return b"\x00\x11\x22\x33\x44\x55\x66\x77\x88\x99\xaa\xbb\x08\x00" + h + payload


def main(d, out):
    frames = []
    ident = 0
    seq = 1
    for line in open(os.path.join(d, "ws.jsonl")):
        j = json.loads(line)
        if j["dir"] != "r2s" or not j["hex"]:
            continue
        data = bytes.fromhex(j["hex"])
        tcp = struct.pack("!HHIIBBHHH", 777, 50000, seq, 1, 5 << 4, 0x18, 65535, 0, 0) + data
        seq += len(data)
        ident += 1
        frames.append((j["t"], ip(RELAY_IP, SERVER_IP, 6, tcp, ident)))
    raw = open(os.path.join(d, "udp.bin"), "rb").read()
    i = 0
    hdr = struct.calcsize("<dB4sHI")
    while i + hdr <= len(raw):
        t, direction, cip, cport, n = struct.unpack_from("<dB4sHI", raw, i)
        i += hdr
        data = raw[i:i + n]
        i += n
        if direction == 0:
            src, sp, dst, dp = cip, cport, SERVER_IP, SERVER_PORT
        else:
            src, sp, dst, dp = SERVER_IP, SERVER_PORT, cip, cport
        udp = struct.pack("!HHHH", sp, dp, 8 + len(data), 0) + data
        ident += 1
        frames.append((t, ip(src, dst, 17, udp, ident)))
    frames.sort(key=lambda f: f[0])
    with open(out, "wb") as f:
        f.write(struct.pack("<IHHiIII", 0xA1B2C3D4, 2, 4, 0, 0, 262144, 1))
        for t, fr in frames:
            sec = int(t)
            f.write(struct.pack("<IIII", sec, int((t - sec) * 1e6), len(fr), len(fr)) + fr)
    print(f"{len(frames)} frames -> {out}")


main(sys.argv[1], sys.argv[2])
