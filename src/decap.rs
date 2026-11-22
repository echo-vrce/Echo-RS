//! Offline decoder for captures made by `tools/record_native.py` and
//! `tools/record_client.py` (keys from its `sessions.jsonl`).
//!
//! Reads `ws.jsonl` (EchoRelay ↔ server, which carries SessionSuccess v5 and
//! so the packet keys) and `udp.bin` (every datagram both ways), decrypts the
//! game traffic with our own transport, and writes one CSV row per message:
//! `dir,t,channel,type,name,mailbox_key,len,hex`.

use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::engine::transport::{SessionCrypto, Transport};
use crate::sns;

fn unhex(s: &str) -> Result<Vec<u8>> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex")).collect()
}

fn hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn key32(v: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 32];
    let n = v.len().min(32);
    k[..n].copy_from_slice(&v[..n]);
    k
}

/// Every SessionSuccess v5 EchoRelay handed the server: one per client.
fn find_keys(dir: &Path) -> Result<Vec<crate::wire::pb::SnsLobbySessionSuccessV5Message>> {
    let mut all = Vec::new();
    // `tools/record_client.py`: each SessionSuccess v5 body the client got.
    if let Ok(text) = fs::read_to_string(dir.join("sessions.jsonl")) {
        for line in text.lines() {
            let j: serde_json::Value = serde_json::from_str(line)?;
            if let Some(h) = j["payload"].as_str() {
                all.push(sns::decode_session_success_v5(&unhex(h)?)?.to_proto());
            }
        }
        if !all.is_empty() {
            return Ok(all);
        }
    }
    for line in fs::read_to_string(dir.join("ws.jsonl"))?.lines() {
        let j: serde_json::Value = serde_json::from_str(line)?;
        if j["dir"] != "r2s" {
            continue;
        }
        let Some(h) = j["hex"].as_str() else { continue };
        let data = unhex(h)?;
        let mut i = 0;
        while i + 24 <= data.len() {
            let sym = u64::from_le_bytes(data[i + 8..i + 16].try_into()?);
            let n = u64::from_le_bytes(data[i + 16..i + 24].try_into()?) as usize;
            let body = data.get(i + 24..i + 24 + n).context("truncated frame")?;
            if sym == sns::sym::SESSION_SUCCESS_V5 {
                all.push(sns::decode_session_success_v5(body)?.to_proto());
            }
            i += 24 + n;
        }
    }
    if all.is_empty() {
        bail!("no SessionSuccess v5 in ws.jsonl");
    }
    Ok(all)
}

/// (client -> server, server -> client) transports for one key set.
fn transports(s: &crate::wire::pb::SnsLobbySessionSuccessV5Message) -> (Transport, Transport) {
    let server = SessionCrypto {
        out_flags: s.server_encoder_flags,
        out_aes: key32(&s.server_enc_key),
        out_mac: key32(&s.server_mac_key),
        out_rnd: key32(&s.server_random_key),
        out_seq: s.server_sequence_id & sns::SEQ_MASK,
        in_flags: s.client_encoder_flags,
        in_aes: key32(&s.client_enc_key),
        in_mac: key32(&s.client_mac_key),
        in_rnd: key32(&s.client_random_key),
        in_seq: s.client_sequence_id & sns::SEQ_MASK,
    };
    // The client's view: what it receives is what the server sent.
    let client = SessionCrypto {
        out_flags: server.in_flags,
        out_aes: server.in_aes,
        out_mac: server.in_mac,
        out_rnd: server.in_rnd,
        out_seq: server.in_seq,
        in_flags: server.out_flags,
        in_aes: server.out_aes,
        in_mac: server.out_mac,
        in_rnd: server.out_rnd,
        in_seq: server.out_seq,
    };
    (Transport::new(&server), Transport::new(&client))
}

pub fn run(dir: &Path, out: &Path) -> Result<()> {
    let keys = find_keys(dir)?;
    let mut used = vec![false; keys.len()];
    // Per client address: its transports, matched to a key set by the first
    // packet that one of them decrypts.
    let mut peers: std::collections::HashMap<(u32, u16), (Transport, Transport)> = std::collections::HashMap::new();

    let raw = fs::read(dir.join("udp.bin"))?;
    let mut w = BufWriter::new(fs::File::create(out)?);
    writeln!(w, "dir,t,channel,type,name,mailbox_key,len,hex,peer")?;
    let (mut i, mut t0, mut undecoded, mut rows) = (0usize, None, [0u32; 2], 0u64);
    const HDR: usize = 8 + 1 + 4 + 2 + 4;
    while i + HDR <= raw.len() {
        let t = f64::from_le_bytes(raw[i..i + 8].try_into()?);
        let dir_c = raw[i + 8];
        let ip = u32::from_be_bytes(raw[i + 9..i + 13].try_into()?);
        let port = u16::from_le_bytes(raw[i + 13..i + 15].try_into()?);
        let n = u32::from_le_bytes(raw[i + 15..i + 19].try_into()?) as usize;
        let data = &raw[i + HDR..i + HDR + n];
        i += HDR + n;
        let t0 = *t0.get_or_insert(t);
        let name = if dir_c == 0 { "c2s" } else { "s2c" };
        let known = peers.get_mut(&(ip, port)).and_then(|pair| {
            let tp = if dir_c == 0 { &mut pair.0 } else { &mut pair.1 };
            tp.receive(data)
        });
        // Nothing yet, or a later session from the same address (new keys):
        // try each unused key set on this packet.
        let r = if known.is_some() {
            known
        } else {
            // Unmatched client: try each unused key set on this packet.
            let mut got = None;
            for (k, s) in keys.iter().enumerate() {
                if used[k] {
                    continue;
                }
                let mut pair = transports(s);
                let tp = if dir_c == 0 { &mut pair.0 } else { &mut pair.1 };
                if let Some(r) = tp.receive(data) {
                    used[k] = true;
                    peers.insert((ip, port), pair);
                    got = Some(r);
                    break;
                }
            }
            got
        };
        let Some(r) = r else {
            undecoded[usize::from(dir_c)] += 1;
            continue;
        };
        let rel = t - t0;
        for (ch, list) in [("rel", &r.reliable), ("mb", &r.mailbox)] {
            for m in list {
                let mname = crate::wire::names::message_name(m.type_).unwrap_or("");
                writeln!(w, "{name},{rel:.4},{ch},{:#018x},{mname},{:#018x},{},{},{port}", m.type_, m.mailbox, m.data.len(), hex(&m.data))?;
                rows += 1;
            }
        }
    }
    w.flush()?;
    println!("{rows} messages -> {}; undecoded packets c2s={} s2c={}", out.display(), undecoded[0], undecoded[1]);
    Ok(())
}

/// A NetRecorder folder: every `part_NNN.bin` in order, as one CSV. In a
/// server's recording received messages are `c2s`, sent ones `s2c`; in a
/// client's (`role client` in info.txt, NetRecorder 1.1) the other way round; `peer` is the recorder's peer index
/// (`-1` local); `channel` is `rel` for a mailbox key of -1, else `mb`. A
/// record marked "same as before" repeats the previous payload of its
/// direction; one with no payload (voice) is written with an empty `hex`.
pub fn netrec(dir: &Path, out: &Path) -> Result<()> {
    let mut parts: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("part_") && n.ends_with(".bin")))
        .collect();
    parts.sort();
    if parts.is_empty() {
        bail!("no part_*.bin in {}", dir.display());
    }
    let mut w = BufWriter::new(fs::File::create(out)?);
    writeln!(w, "dir,t,channel,type,name,mailbox_key,len,hex,peer")?;
    let mut rows = 0u64;
    const HEAD: usize = 8 + 1 + 1 + 2 + 4 + 8 + 8 + 4;
    let client = fs::read_to_string(dir.join("info.txt")).is_ok_and(|s| s.lines().any(|l| l.trim() == "role client"));
    let labels = if client { ["s2c", "c2s"] } else { ["c2s", "s2c"] };
    for part in &parts {
        let raw = fs::read(part)?;
        if raw.len() < 16 || &raw[..8] != b"ERSNETR1" {
            bail!("{} is not a NetRecorder part", part.display());
        }
        let mut last: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
        let mut i = 16;
        while i + HEAD <= raw.len() {
            let r = &raw[i..];
            let t = u64::from_le_bytes(r[0..8].try_into()?);
            let dir_c = (r[8] & 1) as usize;
            let flags = r[9];
            let peer = u32::from_le_bytes(r[12..16].try_into()?);
            let ty = u64::from_le_bytes(r[16..24].try_into()?);
            let key = i64::from_le_bytes(r[24..32].try_into()?);
            let len = u32::from_le_bytes(r[32..36].try_into()?) as usize;
            let stored = flags == 0;
            if stored && i + HEAD + len > raw.len() {
                break; // the part was cut off mid-record (the server stopped)
            }
            let data: &[u8] = if stored {
                last[dir_c] = r[HEAD..HEAD + len].to_vec();
                &last[dir_c]
            } else if flags & 2 != 0 {
                &last[dir_c]
            } else {
                &[]
            };
            i += HEAD + if stored { len } else { 0 };
            let name = crate::wire::names::message_name(ty).unwrap_or("");
            let ch = if dir_c == 0 { "rx" } else if key == -1 { "rel" } else { "mb" };
            let peer = if peer == u32::MAX { -1 } else { i64::from(peer) };
            writeln!(
                w,
                "{},{:.4},{ch},{ty:#018x},{name},{:#018x},{len},{},{peer}",
                labels[dir_c],
                t as f64 / 1e6,
                key as u64,
                hex(data)
            )?;
            rows += 1;
        }
    }
    w.flush()?;
    println!("{rows} messages from {} parts -> {}", parts.len(), out.display());
    Ok(())
}
