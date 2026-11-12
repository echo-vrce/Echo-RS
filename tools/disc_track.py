"""Disc physics over time from a decoded capture (msgs.csv).

Usage: disc_track.py msgs.csv [t_from] [t_to] [step_s]
Prints dir, time, mailbox key, owner, words 8..32, and the decoded x/y/depth
of every SR15NetPhysicsSimStateEvent for the disc, sampled every step_s.
"""
import csv, struct, sys

BIT, WIDTH, BIAS = 267, 19, 250_000


def field(p, bit):
    v = 0
    for k in range(WIDTH):
        i = bit + k
        v |= ((p[i >> 3] >> (i & 7)) & 1) << k
    return v - BIAS


t0 = float(sys.argv[2]) if len(sys.argv) > 2 else 0
t1 = float(sys.argv[3]) if len(sys.argv) > 3 else 1e9
step = float(sys.argv[4]) if len(sys.argv) > 4 else 0.25
last = {}
for r in csv.DictReader(open(sys.argv[1])):
    if r["name"] != "SR15NetPhysicsSimStateEvent":
        continue
    t = float(r["t"])
    if not t0 <= t <= t1:
        continue
    p = bytes.fromhex(r["hex"])
    if len(p) != 64 or struct.unpack_from("<I", p, 4)[0] != 0x428EBC4B:
        continue
    d = r["dir"]
    if t - last.get(d, -1e9) < step:
        continue
    last[d] = t
    owner = struct.unpack_from("<H", p, 0)[0]
    words = struct.unpack_from("<6I", p, 8)
    xyz = [field(p, BIT + i * WIDTH) for i in range(3)]
    print(f"{d} {t:9.3f} key={r['mailbox_key']} own={owner:#06x} w={' '.join(f'{w:08x}' for w in words)} xyz={xyz} raw={p[32:].hex()}")
