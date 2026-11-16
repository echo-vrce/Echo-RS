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
fn find_keys(dir: &Path) -> Result<Vec<nevr_proto::rtapi::v1::SnsLobbySessionSuccessV5Message>> {
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
fn transports(s: &nevr_proto::rtapi::v1::SnsLobbySessionSuccessV5Message) -> (Transport, Transport) {
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
                let mname = rad_protocol::message_registry::lookup_message_name(m.type_).unwrap_or("");
                writeln!(w, "{name},{rel:.4},{ch},{:#018x},{mname},{:#018x},{},{},{port}", m.type_, m.mailbox, m.data.len(), hex(&m.data))?;
                rows += 1;
            }
        }
    }
    w.flush()?;
    println!("{rows} messages -> {}; undecoded packets c2s={} s2c={}", out.display(), undecoded[0], undecoded[1]);
    Ok(())
}
