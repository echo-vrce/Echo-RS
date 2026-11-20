"""Builds what a client needs to load each Combat map from this server.

    python -I tools/gen_combat_maps.py <echo _data/5932408047/rad15/win10> <balance json dir>

- IdsResponse: native's `CR15NetIdCS::IdsRequestCB` answers with one actor id
  per net component of the level, in component order, then the handles. The
  component table is the level's resource of type 0xc29715d62e039402 (32-byte
  rows, actor id first). Native names an actor by its primary name, which for
  13 of Gauss's 352 differs from the row's name, but the client resolves either
  (`CncaGameSpace::Actor` binary-searches every name of the level), and for
  Gauss and the arena the rows give the same actors in the same order as the
  captured responses. Handles run on from the player space's 27 (0x1b..).
- Balance: the captured Gauss block (EchoVRCE's combat server) with the map's
  own `mpl_combat_<map>_map_settings` in place of Gauss's.
- Replicated vars: tools/gen_combat_vars.py, from each map's own recording.

- Payload track (Gauss, Fission): the level's `CR15TrackPointCR` resource
  (type 0x451830f92ddc5cc6; 56-byte rows from 0x40: actor, ..., flags u32 at
  +16, next actor at +32) chained from the point nothing links to. The track
  points are the level's first net components, so point k is trackpoint
  `0x1b + k` on the wire and the payload is the component right after them.
  Flags 0x60 mark the checkpoints (Gauss's fall at the recorded 78 m and
  126 m), 0x10 the end. Positions come from the level's transform table
  (48-byte rows: quat, position, scale, indexed like the name table) in the
  resource of type 0x347869ce492dc7da. Its offset is not in the header: it is
  the one run of exactly `count` (u64 at 0xa8) rows whose rotations are unit
  quaternions (unique on all four maps; Gauss's points equal the recorded
  ones, Fission's first is where its payload actor sits).
- Capture volume (Dyson, Combustion): `CR15NetCaptureVolumeCR` (type
  0x93a1c9eaaaa00b44) lists its shapes from 0x98 as `[id u64][kind u64]`, kind
  9 a sphere and 10 an oriented box (`CncaGameSpace::IsWithinShape`). The
  shapes are in the level's resource 0xe8e38d7781a338a6: per kind a sorted
  `(id i64, index u64)` table, a second table, then the data (sphere: centre,
  radius; box: centre, rotation quaternion, half extents), in world space.

Writes src/engine/data/combat_<map>_{ids,balance}.hex (the vars are gen_combat_vars.py's),
combat_<map>_track.txt, combat_<map>_capture.txt, combat_<map>_barriers.txt,
combat_fission_climax_barriers.txt and lobby_combat_barriers.txt.
"""
import math
import json
import re
import struct
import sys
from pathlib import Path

import zstandard

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "echoquest-combat" / "tools"))
import echopkg  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "src" / "engine" / "data"
GAUSS = ROOT / "captures" / "client-combat-20261116-b" / "extract"
COMPONENTS = 0xC29715D62E039402
MAPS = ["dyson", "combustion", "fission", "gauss"]
FIRST_HANDLE = 0x1B
TRACK_POINTS = 0x451830F92DDC5CC6
NAMES = 0x347869CE492DC7DA
CAPTURE_VOLUME = 0x93A1C9EAAAA00B44
SHAPES = 0xE8E38D7781A338A6
TRACKS = ["gauss", "fission"]
CAPTURES = ["dyson", "combustion"]
DAMAGEABLES = 0xBD1868F576836696
TEMPLATE_SLOTS = 15
# A barrier's box in its actor's frame, by its health (half extents, then the
# box centre's z): where bullets hit it in the recordings. Dyson's 101s: x
# +-1.34, y +-0.7, z 0..0.26 (native's explosion points sit on those edges);
# Dyson's 180s x -2.4..1.93, y -1.22..0.1; Gauss's 230s x -0.9..0.81,
# y -0.49..0.34 (few hits: taken a little wider).
BARRIER_BOXES = {101: (1.34, 0.7, 0.13, 0.13), 180: (2.4, 1.25, 0.15, 0.0), 230: (1.5, 1.0, 0.15, 0.0)}


def unhex(p: Path) -> bytes:
    return bytes.fromhex("".join(c for c in p.read_text() if c in "0123456789abcdefABCDEF"))


def write_hex(p: Path, b: bytes) -> None:
    p.write_text(b.hex() + "\n")


def ids_response(m: echopkg.Manifest, level: str) -> bytes:
    sym = echopkg.sym(level)
    t = m.get(COMPONENTS, sym)
    n = struct.unpack_from("<Q", t, 0x28)[0]
    ids = [t[64 + 32 * i:72 + 32 * i] for i in range(n)]
    out = struct.pack("<Q", sym) + b"".join(ids)
    out += b"".join(struct.pack("<H", FIRST_HANDLE + i) for i in range(n))
    return out


def name_index(t: bytes) -> dict:
    """Actor id -> index: the sorted (id i64, index u64) table near the start."""
    n = struct.unpack_from("<Q", t, 0x28)[0]
    for base in range(0x40, 0x2000, 8):
        if base + 16 * n > len(t):
            break
        v = [struct.unpack_from("<qQ", t, base + 16 * k) for k in range(n)]
        if all(v[k][0] < v[k + 1][0] for k in range(n - 1)) and all(x < 0x10000 for _, x in v):
            return {i & (2**64 - 1): x for i, x in v}
    raise ValueError("no name table")


def transforms(t: bytes) -> int:
    """Start of the transform table: the run of `count` unit-rotation rows."""
    n = struct.unpack_from("<Q", t, 0xA8)[0]
    found = []
    for r in range(0, 48, 4):
        rows = (len(t) - r) // 48
        run = 0
        for i in range(rows):
            q = struct.unpack_from("<4f", t, r + 48 * i)
            run = run + 1 if abs(sum(x * x for x in q) - 1) < 1e-3 else 0
            if run >= n:
                found.append(r + 48 * (i - n + 1))
    assert len(found) == 1, found
    return found[0]


def capture(m: echopkg.Manifest, name: str) -> str:
    level = echopkg.sym(f"mpl_combat_{name}")
    cr = m.get(CAPTURE_VOLUME, level)
    shapes = [(cr[o:o + 8], struct.unpack_from("<Q", cr, o + 8)[0]) for o in range(0x98, len(cr) - 15, 16)]
    d = m.get(SHAPES, level)
    q = struct.unpack_from(f"<{0x570 // 8}Q", d, 0)
    # Section descriptors: [bytes][0][1 << 32][0x20][count][count].
    secs = [(q[i], q[i + 4]) for i in range(len(q) - 5)
            if q[i + 1] == 0 and q[i + 2] == 1 << 32 and q[i + 3] == 0x20 and q[i + 4] == q[i + 5]]
    lines = ["# sphere x y z radius | box x y z qx qy qz qw hx hy hz (world, y up)"]
    for sid, kind in shapes:
        stride, word = {9: (16, "sphere"), 10: (40, "box")}[kind]
        # A kind's sections: keys, a second table (16 bytes a row), the data.
        (n,) = {secs[i][1] for i in range(len(secs) - 2)
                if secs[i][1] and secs[i][1] == secs[i + 1][1] == secs[i + 2][1]
                and secs[i][0] == secs[i + 1][0] == 16 * secs[i][1] and secs[i + 2][0] == stride * secs[i][1]}
        keys = None
        for o in range(0x570, len(d) - 16 * n, 8):
            v = [struct.unpack_from("<qQ", d, o + 16 * i) for i in range(n)]
            if all(v[i][0] < v[i + 1][0] for i in range(n - 1)) and all(x < n for _, x in v) and \
                    any(k & (2**64 - 1) == int.from_bytes(sid, "little") for k, _ in v):
                keys = o
                break
        assert keys is not None, (name, sid.hex())
        idx = dict((k & (2**64 - 1), x) for k, x in (struct.unpack_from("<qQ", d, keys + 16 * i) for i in range(n)))
        data = keys + 2 * 16 * n
        f = struct.unpack_from(f"<{stride // 4}f", d, data + stride * idx[int.from_bytes(sid, "little")])
        lines.append(word + " " + " ".join(f"{x:.4f}" for x in f))
    return "\n".join(lines) + "\n"


def track(m: echopkg.Manifest, name: str) -> str:
    level = echopkg.sym(f"mpl_combat_{name}")
    d = m.get(TRACK_POINTS, level)
    n = struct.unpack_from("<Q", d, 0x28)[0]
    rows = {d[64 + 56 * i:72 + 56 * i]: d[64 + 56 * i:120 + 56 * i] for i in range(n)}
    nexts = {r[32:40] for r in rows.values()}
    (a,) = [k for k in rows if k not in nexts]
    order = []
    while a in rows and a not in order:
        order.append(a)
        a = rows[a][32:40]
    assert len(order) == n, (name, len(order), n)
    comps = m.get(COMPONENTS, level)
    assert [comps[64 + 32 * i:72 + 32 * i] for i in range(n)] == order, "track points are the first components"
    t = m.get(NAMES, level)
    idx = name_index(t)
    start = transforms(t)
    lines = ["# trackpoint metres x y z flags (0x60 checkpoint, 0x10 end)"]
    dist, prev = 0.0, None
    for k, a in enumerate(order):
        o = start + 48 * idx[int.from_bytes(a, "little")]
        q = struct.unpack_from("<4f", t, o)
        pos = struct.unpack_from("<3f", t, o + 16)
        scale = struct.unpack_from("<3f", t, o + 28)
        assert abs(sum(x * x for x in q) - 1) < 1e-3 and all(abs(x - 1) < 1e-3 for x in scale), (name, k)
        if prev:
            dist += math.dist(prev, pos)
        prev = pos
        flags = struct.unpack_from("<I", rows[a], 16)[0]
        lines.append(f"{FIRST_HANDLE + k} {dist:.3f} {pos[0]:.3f} {pos[1]:.3f} {pos[2]:.3f} 0x{flags:x}")
    return "\n".join(lines) + "\n"


def barriers(m: echopkg.Manifest, level: str, ids: bytes) -> str:
    """The level's damageable net components (barriers): pool slot, net handle,
    max health, respawn seconds (0: stays down).

    `CR15NetDamageableCR` (type 0xbd1868f576836696) has 56-byte rows from 0x40:
    actor at +0, max health f32 at +28, a respawn flag f32 at +36 (1: it
    respawns). Native respawns every one flagged after 30 s, whatever +32 says
    (Dyson's 101-health barriers have 1 there and came back after 30.0 s in the
    recording; Gauss's have 30). Pool slots run
    over the rows in order; an ordnance template (bytes 8..12 ffff0f00) takes
    15 (Gauss's first barrier, component 329, is pool slot 0x69 in the
    recording). The handle is the actor's in the level's IdsResponse.
    """
    d = m.get(DAMAGEABLES, echopkg.sym(level))
    n = struct.unpack_from("<Q", d, 0x28)[0]
    count = (len(ids) - 8) // 10
    handle = {ids[8 + 8 * i:16 + 8 * i]: struct.unpack_from("<H", ids, 8 + 8 * count + 2 * i)[0] for i in range(count)}
    t = m.get(NAMES, echopkg.sym(level))
    idx = name_index(t)
    start = transforms(t)
    lines = ["# pool handle max_health respawn_seconds x y z qx qy qz qw half_x half_y half_z offset_z"]
    pool = 0
    for i in range(n):
        r = d[0x40 + 56 * i:0x40 + 56 * (i + 1)]
        template = r[8:12] == b"\xff\xff\x0f\x00"
        if not template and r[:8] in handle:
            mh, _, flag = struct.unpack_from("<fff", r, 28)
            o = start + 48 * idx[int.from_bytes(r[:8], "little")]
            q = struct.unpack_from("<4f", t, o)
            pos = struct.unpack_from("<3f", t, o + 16)
            box = BARRIER_BOXES.get(round(mh), BARRIER_BOXES[230])
            nums = " ".join(f"{x:.4f}" for x in (*pos, *q, *box))
            lines.append(f"{pool} {handle[r[:8]]} {mh:g} {30 if flag else 0} {nums}")
        pool += TEMPLATE_SLOTS if template else 1
    return "\n".join(lines) + "\n"


def decode_block(body: bytes):
    size = struct.unpack_from("<I", body, 0)[0]
    raw = zstandard.ZstdDecompressor().decompress(body[4:], max_output_size=size)
    n = struct.unpack_from("<Q", raw, 0)[0]
    base = 8 + 4 * (n - 1)
    bounds = [0] + [struct.unpack_from("<I", raw, 8 + 4 * i)[0] for i in range(n - 1)] + [len(raw) - base]
    docs = [raw[base + bounds[i]:base + bounds[i + 1]].rstrip(b"\0") for i in range(n)]
    index = docs[-1].decode()
    return [(int(index[16 * i:16 * i + 16], 16), docs[i]) for i in range(n - 1)]


def encode_block(docs) -> bytes:
    index = "".join(f"{i:016X}" for i, _ in docs).encode()
    parts = [d + b"\0" for _, d in docs] + [index + b"\0"]
    raw = struct.pack("<Q", len(parts))
    end = 0
    for p in parts[:-1]:
        end += len(p)
        raw += struct.pack("<I", end)
    raw += b"".join(parts)
    return struct.pack("<I", len(raw)) + zstandard.ZstdCompressor(level=19).compress(raw)


def clean_json(text: str) -> bytes:
    """The game's lenient JSON (// comments, trailing commas) as plain JSON."""
    text = re.sub(r"//[^\n]*", "", text)
    text = re.sub(r",(\s*[}\]])", r"\1", text)
    return json.dumps(json.loads(text), separators=(",", ":")).encode()


def main() -> None:
    m = echopkg.Manifest(Path(sys.argv[1]), "48037dc70b0ecab2")
    balance_dir = Path(sys.argv[2])
    gauss_docs = decode_block(unhex(GAUSS / "balance_gauss.hex"))
    gauss_settings = echopkg.sym("mpl_combat_gauss_map_settings")
    for name in MAPS:
        level = f"mpl_combat_{name}"
        if name == "gauss":
            # Gauss is captured: native's own bytes.
            write_hex(DATA / f"combat_{name}_ids.hex", unhex(GAUSS / "ids_response_gauss.hex"))
            write_hex(DATA / f"combat_{name}_balance.hex", unhex(GAUSS / "balance_gauss.hex"))
        else:
            write_hex(DATA / f"combat_{name}_ids.hex", ids_response(m, level))
            settings = f"{level}_map_settings"
            doc = clean_json((balance_dir / f"{settings}.json").read_text())
            docs = [(echopkg.sym(settings), doc) if i == gauss_settings else (i, d) for i, d in gauss_docs]
            write_hex(DATA / f"combat_{name}_balance.hex", encode_block(docs))
        if name in TRACKS:
            (DATA / f"combat_{name}_track.txt").write_text(track(m, name))
        if name in CAPTURES:
            (DATA / f"combat_{name}_capture.txt").write_text(capture(m, name))
        (DATA / f"combat_{name}_barriers.txt").write_text(barriers(m, level, unhex(DATA / f"combat_{name}_ids.hex")))
        print(level, hex(echopkg.sym(level)), "components", (len(ids_response(m, level)) - 8) // 10)
    # Fission's climax sublevel has its own damageables (1000 health), in its own space.
    climax = "mpl_combat_fission_climax"
    (DATA / "combat_fission_climax_barriers.txt").write_text(barriers(m, climax, unhex(DATA / "combat_fission_climax_ids.hex")))
    lobby = barriers(m, "mpl_lobby_b_combat", unhex(DATA / "ids_response_2645.hex"))
    (DATA / "lobby_combat_barriers.txt").write_text(lobby)


if __name__ == "__main__":
    main()
