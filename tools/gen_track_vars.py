"""Level vars a payload map's scripts set as the payload first reaches a trackpoint.

Native's server runs the level scripts; this server does not, so what they do
with replicated vars is learned from a native server recording (NetRecorder,
decoded with `echo-rust-server --decode-netrec <dir>`).

A var is taken when EVERY change it makes in the recording comes right after a
`SR15NetPayloadTrackpointReachedReliable` (new ground): the checkpoint counter,
the checkpoint symbol, the level's load progression and the flags and floats
that follow it. Vars the server drives itself are left out.

Writes `src/engine/data/combat_<map>_track_vars.txt`:
    <trackpoint> <var> <value> <value before the match>
(hex; a var that did not exist before has 0 as its last column).

usage: gen_track_vars.py <map> <server recording msgs.csv>
"""
import csv
import struct
import sys

csv.field_size_limit(10 ** 9)

# Driven by combat.rs (match flow, payload HUD), or per frame.
SKIP = {
    0x2ed07b1c8c27548d, 0x1c97052a21e09f11,  # red / blue score
    0x0eae82180440d42d, 0xf5bb086c1435b529, 0x5ef77e9eacc127c0,  # round over, celebration, post match
    0xb2996ad399fdc0ff, 0x9de606e496cee8a3, 0x16940a2d58ba1e0a,  # game state, spawn access, team room
    0x3e766a9afe6c97b0, 0xb2996ad484f1d1e8,  # countdown, clock
    0x8ba9857e5c38d27d, 0x24ee4033c21054ed, 0xa8fc5db51329d7ad,  # claim state (+ per level copy)
    0x70abe5363113cdf7, 0xc78afe76b0734e3a, 0xd52dae83702e6fb3,  # payload state, direction, moving
    0xa1570bd3e2eaaac7, 0x21e8feec3e844a3d, 0x5faf3b262449647a,  # attacker / defender claim
    0x9d89a48a426ba48f, 0xe68b8118ed84d0de,  # overtime progress, Gauss door count
}
BEFORE, AFTER = 0.05, 0.20


def main(name, path):
    hist, reached, peer, first = {}, [], None, None
    with open(path, encoding='utf-8', errors='replace') as fh:
        for r in csv.DictReader(fh):
            if r['dir'] != 's2c':
                continue
            n, t = r['name'], float(r['t'])
            if peer is None and 'ReplicatedDeltaUnreliableVars' in n:
                peer, first = r.get('peer', ''), t
            if r.get('peer', '') != peer:
                continue
            if 'ReplicatedDeltaUnreliableVars' in n:
                b = bytes.fromhex(r['hex'])
                for o in range(30, len(b) - 15, 16):
                    h, v = struct.unpack_from('<QQ', b, o)
                    hh = hist.setdefault(h, [])
                    if not hh or hh[-1][1] & 0xffffffff != v & 0xffffffff:
                        hh.append((t, v))
            elif 'PayloadTrackpointReached' in n:
                _, tp = struct.unpack_from('<HH', bytes.fromhex(r['hex']))
                reached.append((t, tp))

    def at(t):
        return next((tp for T, tp in reached if T - BEFORE <= t <= T + AFTER), None)

    rows = []
    for h, hh in hist.items():
        if h in SKIP:
            continue
        existed = hh[0][0] <= first + 1.0
        changes = hh[1:] if existed else hh
        if not changes or len(changes) > 8:
            continue
        tps = [at(t) for t, _ in changes]
        if any(tp is None for tp in tps):
            continue
        initial = hh[0][1] if existed else 0
        rows += [(tp, h, v, initial) for tp, (_, v) in zip(tps, changes)]
    rows.sort()
    out = f'src/engine/data/combat_{name}_track_vars.txt'
    with open(out, 'w', newline='\n') as f:
        f.write(f'# {name}: level vars set when the payload first reaches a trackpoint (tools/gen_track_vars.py).\n')
        f.write('# trackpoint var value before\n')
        for tp, h, v, initial in rows:
            f.write(f'{tp:x} {h:016x} {v:016x} {initial:016x}\n')
    print(out, len(rows), 'rows,', len({r[1] for r in rows}), 'vars, trackpoints', sorted({r[0] for r in rows}))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
