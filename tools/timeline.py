"""Timeline of a decoded capture (msgs.csv from --decode-capture).

Prints replicated-var changes (all vars, or the named ones with --known),
s2c component events (runs of the same event collapsed), and every other
reliable message except the high-rate ones.
"""
import csv, re, struct, sys

msg_rs = open(sys.argv[2] if len(sys.argv) > 2 else "src/engine/msg.rs", encoding="utf-8").read()
names = {}
for m in re.finditer(r"pub const (\w+): u64 = (0x[0-9a-f_]+);(?:\s*//\s*(\w+))?", msg_rs):
    names[int(m.group(2).replace("_", ""), 16)] = m.group(3) or m.group(1)
arr = re.search(r"ST_GOAL: \[u64; 9\] = \[(.*?)\];", msg_rs, re.S).group(1)
for i, h in enumerate(re.findall(r"0x[0-9a-f_]+", arr)):
    names.setdefault(int(h.replace("_", ""), 16), f"ST_GOAL[{i}]")
arr = re.search(r"ST_MATCH_END: \[u64; 7\] = \[(.*?)\];", msg_rs, re.S).group(1)
for h, n in re.findall(r"(0x[0-9a-f_]+), // (\w+)", arr):
    names[int(h.replace("_", ""), 16)] = n


def nm(h):
    return names.get(h, f"{h:#018x}")


SKIP = {"SR15NetComponentEvent", "SR15NetReplicatedDeltaUnreliableVars", "SR15NetServerFrameMsg",
        "SR15NetUserPingEstMsg", "SR15NetClientPingResponseMsg", "SR15NetReplicatedDeltaAck",
        "SR15NetPhysicsSimStateEvent", "SR15NetPlayerStateMsg", "SR15NetTouchStateEvent",
        "SNSLobbyUpdatePings", "SBroadcasterPingUpdate", "SR15NetPlayEmote"}
var_state = {}
last_ev = None
run = 0
only = set(sys.argv[3].split(",")) if len(sys.argv) > 3 else None


def flush_run():
    global last_ev, run
    if last_ev and run > 1:
        print(f"{'':>9}   ... x{run} until {last_ev[1]}")


for r in csv.DictReader(open(sys.argv[1])):
    t, name, d = r["t"], r["name"], r["dir"]
    data = bytes.fromhex(r["hex"])
    if name == "SR15NetReplicatedDeltaUnreliableVars" and d == "s2c":
        stream = struct.unpack_from("<I", data, 8)[0]
        cnt = struct.unpack_from("<H", data, 28)[0]
        for i in range(cnt):
            h, v = struct.unpack_from("<QQ", data, 30 + 16 * i)
            if only and nm(h) not in only:
                continue
            if var_state.get(h) != v:
                lo = v & 0xFFFFFFFF
                f = struct.unpack("<f", struct.pack("<I", lo))[0]
                if nm(h) in ("VAR_GAME_CLOCK", "VAR_COUNTDOWN") and h in var_state:
                    old = struct.unpack("<f", struct.pack("<I", var_state[h] & 0xFFFFFFFF))[0]
                    var_state[h] = v
                    if abs(old - f) < 0.25:
                        continue
                print(f"{t:>9} VAR s{stream} {nm(h)} = {v:#018x} (lo u32 {lo} f32 {f:.3f})")
                var_state[h] = v
    elif name == "SR15NetComponentEvent":
        for i in range(0, len(data) - 23, 24):
            sym, mod, handle, filt = struct.unpack_from("<QQHH", data, i)
            key = (d, sym, mod, handle, filt)
            if last_ev and last_ev[0] == key:
                run += 1
                last_ev = (key, t)
                continue
            flush_run()
            last_ev, run = (key, t), 1
            print(f"{t:>9} EV {d} {nm(sym)} mod={nm(mod)} handle={handle:#06x} filter={filt}")
    elif name not in SKIP or d == "c2s" and name in ("SR15NetComponentEvent",):
        print(f"{t:>9} MSG {d} {r['channel']} {name or r['type']} len={r['len']} {r['hex'][:96]}")
flush_run()
