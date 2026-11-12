//! Offline decoder for captures made by `tools/record_native.py`.
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

fn find_keys(dir: &Path) -> Result<nevr_proto::rtapi::v1::SnsLobbySessionSuccessV5Message> {
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
                return Ok(sns::decode_session_success_v5(body)?.to_proto());
            }
            i += 24 + n;
        }
    }
    bail!("no SessionSuccess v5 in ws.jsonl")
}

pub fn run(dir: &Path, out: &Path) -> Result<()> {
    let s = find_keys(dir)?;
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
    let mut c2s = Transport::new(&server);
    let mut s2c = Transport::new(&client);

    let raw = fs::read(dir.join("udp.bin"))?;
    let mut w = BufWriter::new(fs::File::create(out)?);
    writeln!(w, "dir,t,channel,type,name,mailbox_key,len,hex")?;
    let (mut i, mut t0, mut undecoded, mut rows) = (0usize, None, [0u32; 2], 0u64);
    const HDR: usize = 8 + 1 + 4 + 2 + 4;
    while i + HDR <= raw.len() {
        let t = f64::from_le_bytes(raw[i..i + 8].try_into()?);
        let dir_c = raw[i + 8];
        let n = u32::from_le_bytes(raw[i + 15..i + 19].try_into()?) as usize;
        let data = &raw[i + HDR..i + HDR + n];
        i += HDR + n;
        let t0 = *t0.get_or_insert(t);
        let (tp, name) = if dir_c == 0 { (&mut c2s, "c2s") } else { (&mut s2c, "s2c") };
        let Some(r) = tp.receive(data) else {
            undecoded[usize::from(dir_c)] += 1;
            continue;
        };
        let rel = t - t0;
        for (ch, list) in [("rel", &r.reliable), ("mb", &r.mailbox)] {
            for m in list {
                let mname = rad_protocol::message_registry::lookup_message_name(m.type_).unwrap_or("");
                writeln!(w, "{name},{rel:.4},{ch},{:#018x},{mname},{:#018x},{},{}", m.type_, m.mailbox, m.data.len(), hex(&m.data))?;
                rows += 1;
            }
        }
    }
    w.flush()?;
    println!("{rows} messages -> {}; undecoded packets c2s={} s2c={}", out.display(), undecoded[0], undecoded[1]);
    Ok(())
}
