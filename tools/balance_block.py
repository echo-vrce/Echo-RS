r"""Decode / rebuild the balance settings block a game server sends each client.

After a client reports `SR15NetBalanceSettingsClientReady`, the server sends
`SR15NetBalanceSettingsMassUpdate`; the client then plays with the server's
values, not its own files'. The body:

    u32   uncompressed size
    zstd  frame, which holds:
        u64       document count N
        u32[N-1]  end offset of each document but the last (from the data start)
        N docs    NUL-terminated text: N-1 JSON documents, then an index:
                  N-1 x 16 upper-case hex digits, the id of each JSON document
                  in the same order (sorted as signed i64)

Each id is CSymbol64 of the game's source file name (rad15\json\balance\<name>.json),
the same id the game's JSON resources are stored under (type 607d858c90268f9a).

    python tools/balance_block.py decode <block.hex|block.bin> <out-dir>
    python tools/balance_block.py encode <dir> <out.hex>

`decode` writes one `<name>.json` per document plus `order.txt` (id and name, in
block order); `encode` reads them back in that order. Documents are kept byte for
byte (key order and spacing included) unless you edit them.
"""

from __future__ import annotations

import os
import struct
import sys

import zstandard

# CSymbol64 of the source file names (verified with EchoCombat's symbol64); the
# three marked * are named from their contents (their file names are unknown).
NAMES = {
    0xD7326DC105AFB472: "spatializer*",
    0xE3830436D1D46DA9: "disc_grab_aim_assist*",
    0xF8D40525C88885D8: "mp_equipment",
    0x0A08FB3274D1619B: "disc_team_colors*",
    0x11A8523ED4724CA7: "mp_weapon_settings",
    0x12220D7860DAFFD8: "mp_arena_punch",
    0x12220D7A60D8F9C3: "mp_arena_rules",
    0x2BA5A5812D1AF21A: "mp_shared_movement",
    0x302043C66F60D3EC: "mp_arena_movement",
    0x5D7CB4D5056FC968: "mp_combat_movement",
    0x02B48C1B4948E0A5: "mp_arena_ai_difficulty",
    0x03F14649ABFB4FA3: "mp_combat_rules",
    0xE885FD337739BEA3: "mpl_combat_fission_map_settings",
    0x158EF86FD84EF3DD: "mpl_combat_gauss_map_settings",
}


def read_body(path: str) -> bytes:
    data = open(path, "rb").read()
    if path.endswith(".hex"):
        data = bytes.fromhex("".join(data.decode().split()))
    return data


def decode(body: bytes) -> list[tuple[int, bytes]]:
    size = struct.unpack_from("<I", body)[0]
    raw = zstandard.ZstdDecompressor().decompress(body[4:], max_output_size=size)
    assert len(raw) == size, (len(raw), size)
    n = struct.unpack_from("<Q", raw)[0]
    ends = list(struct.unpack_from(f"<{n - 1}I", raw, 8))
    base = 8 + 4 * (n - 1)
    starts = [0] + ends
    ends = ends + [len(raw) - base]
    docs = [raw[base + s : base + e] for s, e in zip(starts, ends)]
    index = docs[-1].rstrip(b"\0").decode()
    ids = [int(index[i : i + 16], 16) for i in range(0, len(index), 16)]
    assert len(ids) == n - 1, (len(ids), n)
    return [(i, d.rstrip(b"\0")) for i, d in zip(ids, docs[:-1])]


def encode(docs: list[tuple[int, bytes]]) -> bytes:
    index = "".join(f"{i:016X}" for i, _ in docs).encode()
    parts = [d + b"\0" for _, d in docs] + [index + b"\0"]
    ends, at = [], 0
    for p in parts[:-1]:
        at += len(p)
        ends.append(at)
    raw = struct.pack("<Q", len(parts)) + struct.pack(f"<{len(ends)}I", *ends) + b"".join(parts)
    return struct.pack("<I", len(raw)) + zstandard.ZstdCompressor(level=19).compress(raw)


def name_of(i: int) -> str:
    return NAMES.get(i, f"{i:016x}").rstrip("*")


def main() -> None:
    if len(sys.argv) != 4 or sys.argv[1] not in ("decode", "encode"):
        sys.exit(__doc__)
    cmd, src, dst = sys.argv[1:]
    if cmd == "decode":
        docs = decode(read_body(src))
        os.makedirs(dst, exist_ok=True)
        with open(os.path.join(dst, "order.txt"), "w") as order:
            for i, d in docs:
                open(os.path.join(dst, name_of(i) + ".json"), "wb").write(d)
                order.write(f"{i:016x} {name_of(i)}\n")
                print(f"{i:016x} {NAMES.get(i, '?'):34} {len(d):6} bytes")
    else:
        docs = []
        for line in open(os.path.join(src, "order.txt")):
            i, name = line.split()
            docs.append((int(i, 16), open(os.path.join(src, name + ".json"), "rb").read().rstrip(b"\0")))
        body = encode(docs)
        open(dst, "w").write(body.hex() + "\n")
        print(f"{len(docs)} documents -> {len(body)} bytes ({struct.unpack_from('<I', body)[0]} uncompressed)")


if __name__ == "__main__":
    main()
