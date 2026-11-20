"""Each Combat map's solid level collision, for the server's line-of-sight test
(a DETONATOR's blast stops at walls, as native's does).

    python -I tools/gen_combat_collision.py <echo _data/5932408047/rad15/win10>

A level's collision is its `CPhysicsResource` (type 0xB7D338793FA37832, named
by the level): `CPhData` = 36-byte header (body count u32 at 32), then each
body = u64 1 + a 648-byte geo header (array counts) + its arrays in
declaration order + an 800-byte scalar block (flags word at +16) + an 80-byte
transform (quaternion, position, scale). Vertices are the first array (12 B),
triangles the second (52 B, the three vertex indices first). This layout is
the EchoVR Map Editor's (`echo_editor/importers/level/collision_decode.py`,
validated there against stock levels). Only "world solid" bodies are kept
(flags bits 12, 18 and 19 set): not query-only volumes, disc-only plates or
team shields. Sublevels are loaded at the level's origin (the client's
log: "offset by (0.000, 0.000, 0.000)").

Writes src/engine/data/combat_<map>_collision.bin (and arena_collision.bin,
every body of mpl_arena_a, for bounce goals): u32 body count, then per
body u32 vertex count, u32 triangle count, the vertices (3 x i16 each, world
space, centimetres) and the triangles (3 x u16 vertex indices each).
"""
import math
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "echoquest-combat" / "tools"))
import echopkg  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "src" / "engine" / "data"
CPHYS = 0xB7D338793FA37832
ORDER = ((8, 12), (24, 52), (40, 48), (56, 40), (176, 84), (192, 12),
         (208, 12), (224, 4), (240, 4), (256, 4), (272, 4), (288, 8),
         (304, 8), (320, 8), (336, 8), (352, 4), (368, 8), (400, 12),
         (416, 48), (432, 8), (448, 64), (464, 160), (480, 2), (496, 4),
         (512, 12), (528, 40), (544, 2), (560, 4), (576, 12), (592, 48),
         (608, 100), (624, 8), (640, 16))
GEO = 648
SCALAR = 800
SOLID = (1 << 12) | (1 << 18) | (1 << 19)
LEVELS = {
    "dyson": ["mpl_combat_dyson"],
    "combustion": ["mpl_combat_combustion"],
    "fission": ["mpl_combat_fission", "mpl_combat_fission_cargobay", "mpl_combat_fission_climax",
                "mpl_combat_fission_pantheon", "mpl_combat_fission_prologue"],
    "gauss": ["mpl_combat_gauss", "mpl_combat_gauss_section01a", "mpl_combat_gauss_section01b",
              "mpl_combat_gauss_section02", "mpl_combat_gauss_section03"],
    # The arena (bounce goals: did the disc turn at a surface?). Every body is
    # kept: the disc also comes off its disc-only plates.
    "arena": ["mpl_arena_a"],
}
ALL_BODIES = {"arena"}


def rotate(q, v):
    x, y, z, w = q
    # v + 2w(q x v) + 2 q x (q x v)
    cx = y * v[2] - z * v[1]
    cy = z * v[0] - x * v[2]
    cz = x * v[1] - y * v[0]
    ccx = y * cz - z * cy
    ccy = z * cx - x * cz
    ccz = x * cy - y * cx
    return (v[0] + 2 * (w * cx + ccx), v[1] + 2 * (w * cy + ccy), v[2] + 2 * (w * cz + ccz))


def bodies(d: bytes):
    if len(d) < 44:
        return
    n = struct.unpack_from("<I", d, 32)[0]
    off = 36
    for _ in range(n):
        assert struct.unpack_from("<Q", d, off)[0] == 1, "not a geo body"
        geo = off + 8
        arrays = {}
        o = geo + GEO
        for coff, stride in ORDER:
            c = struct.unpack_from("<I", d, geo + coff)[0]
            arrays[coff] = (o, c)
            o += c * stride
        scalar = o
        flags = struct.unpack_from("<I", d, scalar + 16)[0]
        x = scalar + SCALAR
        q = struct.unpack_from("<4f", d, x)
        pos = struct.unpack_from("<3f", d, x + 16)
        scale = struct.unpack_from("<3f", d, x + 28)
        vo, vc = arrays[8]
        to, tc = arrays[24]
        verts = [struct.unpack_from("<3f", d, vo + 12 * i) for i in range(vc)]
        tris = [struct.unpack_from("<3I", d, to + 52 * i) for i in range(tc)]
        yield flags, q, pos, scale, verts, tris
        off = scalar + SCALAR + 80


def main() -> None:
    m = echopkg.Manifest(Path(sys.argv[1]), "48037dc70b0ecab2")
    for name, levels in LEVELS.items():
        out = []
        tri_total = 0
        lo, hi = [1e9] * 3, [-1e9] * 3
        for level in levels:
            d = m.get(CPHYS, echopkg.sym(level))
            if not d:
                continue
            for flags, q, pos, scale, verts, tris in bodies(d):
                kept = flags & SOLID == SOLID or name in ALL_BODIES
                print(f"  {level}: flags {flags:08x} verts {len(verts)} tris {len(tris)}{'' if kept else ' (skipped)'}")
                if not kept or not tris:
                    continue
                assert len(verts) < 65536
                if not all(math.isfinite(s) and s != 0 for s in scale):
                    scale = (1.0, 1.0, 1.0)
                world = [tuple(p + r for p, r in zip(pos, rotate(q, (v[0] * scale[0], v[1] * scale[1], v[2] * scale[2])))) for v in verts]
                b = struct.pack("<II", len(world), len(tris))
                for v in world:
                    cm = [round(c * 100) for c in v]
                    assert all(-32768 <= c < 32768 for c in cm), v
                    b += struct.pack("<3h", *cm)
                    lo = [min(a, c) for a, c in zip(lo, v)]
                    hi = [max(a, c) for a, c in zip(hi, v)]
                for t in tris:
                    b += struct.pack("<3H", *t)
                out.append(b)
                tri_total += len(tris)
        blob = struct.pack("<I", len(out)) + b"".join(out)
        (DATA / (f"{name}_collision.bin" if name in ALL_BODIES else f"combat_{name}_collision.bin")).write_bytes(blob)
        print(name, tri_total, "triangles", len(blob), "bytes", "bounds", [round(v, 1) for v in lo], [round(v, 1) for v in hi])

if __name__ == "__main__":
    main()
