"""Builds each Combat map's replicated vars from a native server recording.

    python -I tools/gen_combat_vars.py

Reads captures/netrec-<map>-20261116-server/<run>/msgs.csv (decoded with
`echo-rust-server --decode-netrec`) and writes, per map:

- src/engine/data/combat_<map>_rep_s0.hex: native's first full
  `SR15NetReplicatedDeltaUnreliableVars` (the earliest joiner's, before the
  match began), plus the vars native only creates later that the server
  drives (`LATE`), at their first recorded value. Native combat has one var
  stream; the level's own scripts own most of these vars (per-actor keys, so
  every map has its own set).
- src/engine/data/combat_<map>_reflex.txt: what native's level scripts do
  with a client's server-only (filter 5) `SR15NetComponentEvent`: the vars
  set, and to what, within two frames of it, in at least 80% of the
  occurrences (doors, the war room's objects, area triggers).
- src/engine/data/combat_<map>_barrier_vars.txt: the vars that go to 0 while
  a barrier is down (one var may follow several barriers).
"""
import collections
import csv
import itertools
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "src" / "engine" / "data"
MAPS = ["dyson", "combustion", "fission", "gauss"]
VARS = "SR15NetReplicatedDeltaUnreliableVars"
BARRIER_EVENT = "ff278203989fcc2b"
WINDOW = 0.03
# Vars the server computes itself (combat.rs): never learned as reflexes.
COMPUTED = {
    0xB2996AD484F1D1E8, 0xB2996AD399FDC0FF, 0x3E766A9AFE6C97B0,  # clock, state, countdown
    0xE9192A0CCDCCF498, 0x34F3A75DBB96C0B3, 0x45000053AE43FE8B, 0xED17977AB05DBD33,
    0xDF542064982BCDF7, 0x4F8AA683CC60B185, 0xA01BC18B5719726B, 0x8AD4206DD36BBC57,
    0xBA4A488EFD29847B, 0x5AA67DAE23FB4993, 0x8BD541A7F8565A03, 0xC3C1C808A6374775,
    0xEB40DD00966D72FE, 0x16940A2D58BA1E0A, 0x9DE606E496CEE8A3, 0x0EAE82180440D42D,
    0x5EF77E9EACC127C0, 0xF5BB086C1435B529, 0x4EB3E1B152A1BA7A, 0x2ED07B1C8C27548D,
    0x1C97052A21E09F11, 0xB546F3ACEB9C95AB, 0x72C158616250640B, 0xC5C2FC48AB2603D7,
}
# Created by native's scripts mid-match; the server sets them (combat.rs).
LATE = {
    0xDF542064982BCDF7, 0x4F8AA683CC60B185, 0xBA4A488EFD29847B, 0x5AA67DAE23FB4993,
    0x8BD541A7F8565A03, 0xC3C1C808A6374775, 0xEB40DD00966D72FE, 0xB546F3ACEB9C95AB,
    0x72C158616250640B, 0xC5C2FC48AB2603D7,
}


# A barrier's var is a CSymbol64 of a name holding its net handle in hex, 34
# characters before the end: the recorded vars of two barriers differ by
# exactly the CRC of their handles' differing hex digits (Dyson 0xb8..0xbd,
# Gauss 0x165, 0x16a..0x16c, the last three from the client recording
# captures/client-combat-20261116-b). So one recorded var gives every
# barrier's: all 13 of Dyson's and 9 of Gauss's computed this way are vars of
# native's baseline, at 1.
SUFFIX = 34
POLY = 0x95AC9329AC4BC9B5
M64 = (1 << 64) - 1
_T = []
for _i in range(256):
    _v = (POLY << 1) & M64 if _i & 0x80 else 0
    if _i & 0x40:
        _v = 0xBEF5B57AF4DC5ADF if _i & 0x80 else POLY
    for _bit in (0x20, 0x10, 8, 4, 2, 1):
        _v = ((_v * 2) ^ POLY) & M64 if _i & _bit else (_v * 2) & M64
    _T.append((_v * 2) & M64)


def barrier_var(known_handle: int, known_var: int, handle: int) -> int:
    a, b = f"{known_handle:04x}".encode(), f"{handle:04x}".encode()
    v = known_var
    for i in range(4):
        d = a[3 - i] ^ b[3 - i]
        for _ in range(SUFFIX + i if d else 0):
            d = (_T[(d >> 56) & 0xFF] ^ ((d << 8) & M64)) & M64
        v ^= d
    return v


def rows(b: bytes):
    return [struct.unpack_from("<QQ", b, o) for o in range(30, len(b) - 15, 16)]


def main() -> None:
    for name in MAPS:
        run = sorted((ROOT / "captures" / f"netrec-{name}-20261116-server").iterdir())[0]
        full = None
        first_seen = {}
        changes = []  # (t, key, value), peer 0's view
        events = []  # client filter-5 events: (t, sym, module, handle)
        barrier = []  # (t, handle) native's barrier events to peer 0
        last = {}
        for r in csv.DictReader(open(run / "msgs.csv")):
            b = bytes.fromhex(r["hex"]) if r["hex"] else b""
            t = float(r["t"])
            if r["dir"] == "s2c" and r["name"] == VARS:
                if full is None and len(b) > 1000:
                    full = b
                if r["peer"] != "0":
                    continue
                for k, v in rows(b):
                    first_seen.setdefault(k, v)
                    if k in last and last[k] != v:
                        changes.append((t, k, v))
                    last[k] = v
            elif r["name"] == "SR15NetComponentEvent" and len(b) >= 20:
                sym, module, handle, filt = b[:8].hex(), b[8:16].hex(), b[16] | b[17] << 8, b[18] | b[19] << 8
                if r["dir"] == "c2s" and filt == 5:
                    events.append((t, sym, module, handle))
                elif r["dir"] == "s2c" and r["peer"] == "0" and sym == BARRIER_EVENT:
                    barrier.append((t, handle))
        # Baseline.
        n = struct.unpack_from("<H", full, 28)[0]
        have = {k for k, _ in rows(full)}
        # At their first value's type tag with the value itself 0 (the server sets it).
        extra = [(k, first_seen[k] & ~0xFFFFFFFF) for k in sorted(LATE) if k in first_seen and k not in have]
        body = bytearray(full[:28]) + struct.pack("<H", n + len(extra)) + full[30:30 + 16 * n]
        for k, v in extra:
            body += struct.pack("<QQ", k, v)
        (DATA / f"combat_{name}_rep_s0.hex").write_text(body.hex() + "\n")
        # Reflexes.
        def after(t):
            # Only the level's own vars: those created later belong to players
            # (their keys carry the slot) or to the server's own state.
            return [(k, v) for (tc, k, v) in changes if t <= tc <= t + WINDOW and k not in COMPUTED and k in have]
        seen = collections.Counter((s, m, h) for _, s, m, h in events)
        follow = collections.defaultdict(collections.Counter)
        for t, s, m, h in events:
            for kv in set(after(t)):
                follow[(s, m, h)][kv] += 1
        lines = ["# sym module handle var value (wire bytes for sym/module; var and value as u64)"]
        for key, cnt in sorted(seen.items()):
            for (k, v), c in sorted(follow[key].items()):
                if c >= 2 and c >= 0.8 * cnt:
                    lines.append(f"{key[0]} {key[1]} {key[2]:#x} {k:016x} {v:016x}")
        (DATA / f"combat_{name}_reflex.txt").write_text("\n".join(lines) + "\n")
        # Barrier vars: a var that only ever changes right after native's
        # barrier events (break, respawn, the next round's restore) is 0 while
        # a barrier of its set is down. The set: the fewest barriers whose
        # events cover every change.
        reflexed = {int(l.split()[3], 16) for l in lines[1:]}
        by_var = collections.defaultdict(list)
        for t, k, v in changes:
            if k in have and k not in COMPUTED and k not in reflexed:
                by_var[k].append(t)
        blines = ["# barrier_handle var (the var is 0 while the barrier is down, else 1)"]
        learned = {}
        for k, ts in sorted(by_var.items()):
            if len(ts) < 2:
                continue
            covers = {h: {t for t in ts if any(0 <= t - tb <= WINDOW for tb, hb in barrier if hb == h)} for h in {h for _, h in barrier}}
            covers = {h: c for h, c in covers.items() if c}
            if not covers or set().union(*covers.values()) != set(ts):
                continue
            best = None
            for size in range(1, len(covers) + 1):
                for s in itertools.combinations(sorted(covers), size):
                    if set().union(*(covers[h] for h in s)) == set(ts):
                        best = s
                        break
                if best:
                    break
            whole = [h for h, c in covers.items() if c == set(ts)]
            if len(whole) == 1:
                learned[whole[0]] = k
        handles = [int(l.split()[1]) for l in (DATA / f"combat_{name}_barriers.txt").read_text().splitlines() if l and not l.startswith("#")]
        if learned:
            h0, k0 = sorted(learned.items())[0]
            for h in handles:
                k = barrier_var(h0, k0, h)
                assert learned.get(h, k) == k, (name, hex(h))
                assert dict(rows(full)).get(k) == 1, (name, hex(h), "not a baseline var at 1")
                blines.append(f"{h:#x} {k:016x}")
        (DATA / f"combat_{name}_barrier_vars.txt").write_text("\n".join(blines) + "\n")
        print(name, run.name, "baseline", n, "+", len(extra), "reflexes", len(lines) - 1, "barrier vars", len(blines) - 1)


if __name__ == "__main__":
    main()
