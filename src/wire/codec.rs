//! The message codec inside a transport frame.
//!
//! A frame body is either *raw* (until compression is switched on) or
//! *encoded*.
//!
//! Raw: per message `[type u64][mailbox key u64, mailbox channel only]
//! [len u64][data]`, each record padded to 8 bytes.
//!
//! Encoded: one bit stream (LSB first in little-endian 64-bit words). A frame
//! header gives the message count and the min/max of each record field, so a
//! record field takes only the bits its range needs. Each record names a
//! message id (the sender's handle for a `(type, mailbox key)` pair, sent in
//! full only the first time), a frame number, the size, the payload's bit
//! length and four mode bits. The payload is a list of 32-bit words, each
//! written as the bit positions that differ from the previous payload of the
//! same id (or from zero), optionally skipping unchanged runs (`partial`) or
//! as an offset from a minimum (`smode`).
//!
//! We send every message under a fresh id, as a whole (non-relative) payload,
//! so the encoder is the simple half; the decoder reads everything clients send.

use std::collections::HashMap;
use std::hash::Hash;

/// The mailbox key of reliable-channel messages.
pub const RELIABLE_KEY: u64 = u64::MAX;

/// Most messages in one frame.
const MAX_MESSAGES: usize = 256;
/// Largest single message payload we accept.
const MAX_MESSAGE_BYTES: usize = 65_536;
/// Message ids remembered per decoder (twice this across both generations).
const ID_CAP: usize = 4_096;
/// Bytes of previous payloads remembered per decoder (per generation).
const BASE_BYTES_CAP: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedMessage {
    pub size_bytes: u32,
    pub type_: u64,
    pub mailbox: u64,
    pub data: Vec<u8>,
}

const fn width(n: u64) -> usize {
    (64 - n.leading_zeros()) as usize
}

// ─── Bits ───────────────────────────────────────────────────────────────────

#[derive(Default)]
struct BitWriter {
    words: Vec<u64>,
    len: usize,
}

impl BitWriter {
    fn bits(&mut self, v: u64, n: usize) {
        if n == 0 {
            return;
        }
        let v = if n < 64 { v & ((1 << n) - 1) } else { v };
        let (w, b) = (self.len / 64, self.len % 64);
        while self.words.len() <= (self.len + n) / 64 {
            self.words.push(0);
        }
        self.words[w] |= v << b;
        if b + n > 64 {
            self.words[w + 1] |= v >> (64 - b);
        }
        self.len += n;
    }

    fn bit(&mut self, v: bool) {
        self.bits(u64::from(v), 1);
    }

    /// `v` in `[base, limit]`: no bits for an empty range, the plain offset
    /// for a range of up to 5 bits, else a width prefix and the offset.
    fn ranged(&mut self, v: u64, base: u64, limit: u64) {
        let delta = v.wrapping_sub(base);
        let mut n = width(limit.wrapping_sub(base));
        if n > 5 {
            let dw = width(delta);
            self.bits(dw as u64, width(n as u64));
            n = dw;
        }
        self.bits(delta, n);
    }

    fn ranged_i32(&mut self, v: i32, base: i32, limit: i32) {
        let range = (i64::from(limit) - i64::from(base)) as u32;
        let delta = (i64::from(v) - i64::from(base)) as u32;
        let mut n = width(u64::from(range));
        if n > 5 {
            let dw = width(u64::from(delta));
            self.bits(dw as u64, width(n as u64));
            n = dw;
        }
        self.bits(u64::from(delta), n);
    }

    fn append(&mut self, other: &BitWriter) {
        let mut left = other.len;
        for &w in &other.words {
            if left == 0 {
                break;
            }
            let n = left.min(64);
            self.bits(w, n);
            left -= n;
        }
    }

    fn into_bytes(self) -> Vec<u8> {
        let mut out: Vec<u8> = self.words.iter().flat_map(|w| w.to_le_bytes()).collect();
        out.truncate(self.len.div_ceil(8));
        out
    }
}

struct BitReader<'a> {
    words: &'a [u64],
    pos: usize,
    end: usize,
}

impl BitReader<'_> {
    /// Up to `n` bits; past the end reads as zeros (and stops there).
    fn bits(&mut self, n: usize) -> u64 {
        let n = n.min(self.end.saturating_sub(self.pos));
        if n == 0 {
            return 0;
        }
        let (w, b) = (self.pos / 64, self.pos % 64);
        let mut v = self.words[w] >> b;
        if b > 0 && b + n > 64 && w + 1 < self.words.len() {
            v |= self.words[w + 1] << (64 - b);
        }
        if n < 64 {
            v &= (1 << n) - 1;
        }
        self.pos += n;
        v
    }

    fn bit(&mut self) -> bool {
        self.bits(1) != 0
    }

    fn ranged(&mut self, base: u64, limit: u64) -> u64 {
        let mut n = width(limit.wrapping_sub(base));
        if n > 5 {
            n = self.bits(width(n as u64)) as usize;
        }
        base.wrapping_add(self.bits(n))
    }

    fn ranged_i32(&mut self, base: i32, limit: i32) -> i32 {
        let mut n = width(u64::from((i64::from(limit) - i64::from(base)) as u32));
        if n > 5 {
            n = self.bits(width(n as u64)) as usize;
        }
        base.wrapping_add(self.bits(n) as u32 as i32)
    }

    fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.end);
    }
}

// ─── Frame and record headers ───────────────────────────────────────────────

/// Field ranges of one encoded frame.
#[derive(Default)]
struct Ranges {
    count: u32,
    id: (u64, u64),
    frame: (u32, u32),
    size: (u32, u32),
    bits: (u32, u32),
    sub_min: (i32, i32),
    sub_off: (u32, u32),
    mailbox: bool,
}

/// Upper limits the header's max fields are measured against.
const FRAME_LIMIT: u64 = 0x3f;
const SIZE_LIMIT: u64 = 0xf_ffff;
const BITS_LIMIT: u64 = 0x7f_fff8;

impl Ranges {
    fn sub_off_limit(&self) -> u64 {
        (i64::from(i32::MAX) - i64::from(self.sub_min.0)) as u64
    }

    fn write(&self, w: &mut BitWriter) {
        let count = u64::from(self.count).saturating_sub(1);
        w.bits(width(count) as u64, 4);
        w.bits(count, width(count));
        w.bits(width(self.id.0) as u64, 7);
        w.bits(self.id.0, width(self.id.0));
        w.ranged(self.id.1, self.id.0, u64::MAX);
        let f = u64::from(self.frame.0);
        w.bits(width(f) as u64, 3);
        w.bits(f, width(f));
        w.ranged(u64::from(self.frame.1), f, FRAME_LIMIT);
        let s = u64::from(self.size.0);
        w.bits(width(s) as u64, 5);
        w.bits(s, width(s));
        w.ranged(u64::from(self.size.1), s, SIZE_LIMIT);
        let b = u64::from(self.bits.0);
        w.bits(width(b) as u64, 5);
        w.bits(b, width(b));
        w.ranged(u64::from(self.bits.1), b, BITS_LIMIT);
        let sm = u64::from((self.sub_min.0 as u32) ^ 0x8000_0000);
        w.bits(width(sm) as u64, 6);
        w.bits(sm, width(sm));
        w.ranged_i32(self.sub_min.1, self.sub_min.0, i32::MAX);
        let limit = self.sub_off_limit();
        if limit != 0 {
            let mut n = width(limit);
            if n > 5 {
                let vw = width(u64::from(self.sub_off.0));
                w.bits(vw as u64, width(n as u64));
                n = vw;
            }
            w.bits(u64::from(self.sub_off.0), n);
        }
        w.ranged(u64::from(self.sub_off.1), u64::from(self.sub_off.0), limit);
    }

    fn read(r: &mut BitReader<'_>, mailbox: bool) -> Self {
        let n = r.bits(4) as usize;
        let count = r.bits(n) as u32 + 1;
        let n = r.bits(7) as usize;
        let id_min = r.bits(n);
        let id_max = r.ranged(id_min, u64::MAX);
        let n = r.bits(3) as usize;
        let frame_min = r.bits(n);
        let frame_max = r.ranged(frame_min, FRAME_LIMIT);
        let n = r.bits(5) as usize;
        let size_min = r.bits(n);
        let size_max = r.ranged(size_min, SIZE_LIMIT);
        let n = r.bits(5) as usize;
        let bits_min = r.bits(n);
        let bits_max = r.ranged(bits_min, BITS_LIMIT);
        let n = r.bits(6) as usize;
        let sub_min_lo = ((r.bits(n) as u32) ^ 0x8000_0000) as i32;
        let sub_min_hi = r.ranged_i32(sub_min_lo, i32::MAX);
        let mut h = Self {
            count,
            id: (id_min, id_max),
            frame: (frame_min as u32, frame_max as u32),
            size: (size_min as u32, size_max as u32),
            bits: (bits_min as u32, bits_max as u32),
            sub_min: (sub_min_lo, sub_min_hi),
            sub_off: (0, 0),
            mailbox,
        };
        let limit = h.sub_off_limit();
        let off_lo = if limit == 0 {
            0
        } else {
            let mut n = width(limit);
            if n > 5 {
                n = r.bits(width(n as u64)) as usize;
            }
            r.bits(n) as u32
        };
        h.sub_off = (off_lo, r.ranged(u64::from(off_lo), limit) as u32);
        h
    }
}

/// One message record of an encoded frame.
struct Record {
    id: u64,
    /// Type and key are sent (first use of this id).
    fresh: bool,
    type_: u64,
    key: u64,
    frame: u32,
    size: u32,
    bits: u32,
    /// Payload is relative to this id's previous payload.
    relative: bool,
    /// Each word is preceded by a changed bit; unchanged words come in runs.
    partial: bool,
    smode: bool,
    xmode: bool,
    sub_min: i32,
    sub_off: u32,
}

impl Record {
    fn write(&self, w: &mut BitWriter, h: &Ranges) {
        w.ranged(self.id, h.id.0, h.id.1);
        w.bit(self.fresh);
        if self.fresh {
            w.bits(width(self.type_) as u64, 7);
            w.bits(self.type_, width(self.type_));
            if h.mailbox {
                w.bits(width(self.key) as u64, 7);
                w.bits(self.key, width(self.key));
            }
        }
        w.ranged(u64::from(self.frame), u64::from(h.frame.0), u64::from(h.frame.1));
        w.ranged(u64::from(self.size), u64::from(h.size.0), u64::from(h.size.1));
        w.ranged(u64::from(self.bits), u64::from(h.bits.0), u64::from(h.bits.1));
        w.bit(self.relative);
        w.bit(self.partial);
        w.bit(self.smode);
        w.bit(self.xmode);
        if self.smode {
            w.ranged_i32(self.sub_min, h.sub_min.0, h.sub_min.1);
            w.ranged(u64::from(self.sub_off), u64::from(h.sub_off.0), u64::from(h.sub_off.1));
        }
    }

    fn read(r: &mut BitReader<'_>, h: &Ranges) -> Self {
        let id = r.ranged(h.id.0, h.id.1);
        let fresh = r.bit();
        let (type_, key) = if fresh {
            let n = r.bits(7) as usize;
            let t = r.bits(n);
            let k = if h.mailbox {
                let n = r.bits(7) as usize;
                r.bits(n)
            } else {
                RELIABLE_KEY
            };
            (t, k)
        } else {
            (0, RELIABLE_KEY)
        };
        let frame = r.ranged(u64::from(h.frame.0), u64::from(h.frame.1)) as u32;
        let size = r.ranged(u64::from(h.size.0), u64::from(h.size.1)) as u32;
        let bits = r.ranged(u64::from(h.bits.0), u64::from(h.bits.1)) as u32;
        let relative = r.bit();
        let partial = r.bit();
        let smode = r.bit();
        let xmode = r.bit();
        let (sub_min, sub_off) = if smode {
            (r.ranged_i32(h.sub_min.0, h.sub_min.1), r.ranged(u64::from(h.sub_off.0), u64::from(h.sub_off.1)) as u32)
        } else {
            (0, 0)
        };
        Self { id, fresh, type_, key, frame, size, bits, relative, partial, smode, xmode, sub_min, sub_off }
    }
}

/// Width of the next step's bit-position delta once `pos` bits are passed.
fn step_width(pos: u32) -> usize {
    if pos >= 31 { 0 } else { width(u64::from(31 - pos)) }
}

/// One payload word as the positions of its set bits (it is XORed onto the
/// previous word, here zero): each a delta from the last, then a more-follows
/// bit. Zero is written as bit 31 twice (which cancels).
fn write_word(w: &mut BitWriter, mut v: u32) {
    if v == 0 {
        w.bits(31, 5);
        w.bit(true);
        w.bit(false);
        return;
    }
    let mut pos = 0u32;
    let mut n = 5;
    let mut any = false;
    while v != 0 {
        let at = v.trailing_zeros();
        w.bits(u64::from(at - pos), n);
        if at == 31 && any {
            break;
        }
        v ^= 1 << at;
        w.bit(v != 0);
        if at == 31 {
            pos = 31;
        } else {
            pos = at + 1;
            any = true;
        }
        n = step_width(pos);
    }
}

fn read_word(r: &mut BitReader<'_>, prev: u32) -> u32 {
    let mut cur = prev;
    let mut pos = 0u32;
    let mut n = 5;
    let mut any = false;
    for _ in 0..=32 {
        let at = pos + r.bits(n) as u32;
        if at > 31 {
            break;
        }
        cur ^= 1 << at;
        if at == 31 {
            if any {
                break;
            }
            pos = 31;
            n = 0;
        } else {
            any = true;
            pos = at + 1;
            n = step_width(pos);
        }
        if !r.bit() {
            break;
        }
    }
    cur
}

// ─── Two-generation map ─────────────────────────────────────────────────────

/// A map bounded in entries and bytes: when the hot generation fills, the
/// cold one is dropped and the hot one becomes cold. A hit in the cold
/// generation moves back to the hot one.
struct TwoGen<K, V> {
    hot: HashMap<K, V>,
    cold: HashMap<K, V>,
    hot_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
    weigh: fn(&V) -> usize,
}

impl<K: Eq + Hash, V> TwoGen<K, V> {
    fn new(max_entries: usize, max_bytes: usize, weigh: fn(&V) -> usize) -> Self {
        Self { hot: HashMap::new(), cold: HashMap::new(), hot_bytes: 0, max_entries, max_bytes, weigh }
    }

    fn get(&mut self, k: &K) -> Option<&V> {
        if !self.hot.contains_key(k) {
            let (k, v) = self.cold.remove_entry(k)?;
            self.insert(k, v);
        }
        self.hot.get(k)
    }

    fn insert(&mut self, k: K, v: V) {
        self.cold.remove(&k);
        if let Some(old) = self.hot.remove(&k) {
            self.hot_bytes -= (self.weigh)(&old);
        }
        let cost = (self.weigh)(&v);
        if self.hot.len() + 1 > self.max_entries || self.hot_bytes + cost > self.max_bytes {
            self.cold = std::mem::take(&mut self.hot);
            self.hot_bytes = 0;
        }
        self.hot_bytes += cost;
        self.hot.insert(k, v);
    }

    fn take(&mut self, k: &K) -> Option<V> {
        if let Some(v) = self.hot.remove(k) {
            self.hot_bytes -= (self.weigh)(&v);
            return Some(v);
        }
        self.cold.remove(k)
    }
}

// ─── Encoder ────────────────────────────────────────────────────────────────

pub struct MessageEncoder {
    encoded: bool,
    encode_next: bool,
    mailbox: bool,
    next_id: u64,
    /// `(id, type, key, data)`.
    staged: Vec<(u64, u64, u64, Vec<u8>)>,
}

impl MessageEncoder {
    /// A reliable-channel (`false`) or mailbox-channel (`true`) encoder.
    pub fn new(mailbox: bool) -> Self {
        Self { encoded: false, encode_next: false, mailbox, next_id: 0, staged: Vec::new() }
    }

    /// Encode from the next frame on (the frame announcing it goes raw).
    pub fn enable_ece(&mut self) {
        self.encode_next = true;
    }

    pub fn begin_frame(&mut self) {
        if std::mem::take(&mut self.encode_next) {
            self.encoded = true;
        }
        self.staged.clear();
    }

    /// Stage one message; `false` once the frame holds 256.
    pub fn encode(&mut self, type_: u64, key: u64, data: &[u8]) -> bool {
        if self.staged.len() >= MAX_MESSAGES {
            return false;
        }
        // Every message takes the next id, raw frames included.
        self.staged.push((self.next_id, type_, key, data.to_vec()));
        self.next_id = self.next_id.wrapping_add(1);
        true
    }

    pub fn end_frame(&mut self) -> Vec<u8> {
        let staged = std::mem::take(&mut self.staged);
        if !self.encoded {
            let mut out = Vec::new();
            for (_, type_, key, data) in &staged {
                out.extend_from_slice(&type_.to_le_bytes());
                let key = if self.mailbox { *key } else { RELIABLE_KEY };
                if key != RELIABLE_KEY {
                    out.extend_from_slice(&key.to_le_bytes());
                }
                out.extend_from_slice(&(data.len() as u64).to_le_bytes());
                out.extend_from_slice(data);
                out.resize(out.len().div_ceil(8) * 8, 0);
            }
            return out;
        }
        if staged.is_empty() {
            return Vec::new();
        }
        let mut h = Ranges {
            count: staged.len() as u32,
            id: (u64::MAX, 0),
            size: (u32::MAX, 0),
            bits: (u32::MAX, 0),
            mailbox: self.mailbox,
            ..Ranges::default()
        };
        let mut items = Vec::with_capacity(staged.len());
        for (id, type_, key, data) in staged {
            let mut payload = BitWriter::default();
            for chunk in data.chunks(4) {
                let mut w = [0u8; 4];
                w[..chunk.len()].copy_from_slice(chunk);
                write_word(&mut payload, u32::from_le_bytes(w));
            }
            let rec = Record {
                id,
                fresh: true,
                type_,
                key,
                frame: 0,
                size: data.len() as u32,
                bits: payload.len as u32,
                relative: false,
                partial: false,
                smode: false,
                xmode: false,
                sub_min: 0,
                sub_off: 0,
            };
            h.id = (h.id.0.min(id), h.id.1.max(id));
            h.size = (h.size.0.min(rec.size), h.size.1.max(rec.size));
            h.bits = (h.bits.0.min(rec.bits), h.bits.1.max(rec.bits));
            items.push((rec, payload));
        }
        let mut w = BitWriter::default();
        h.write(&mut w);
        for (rec, payload) in &items {
            rec.write(&mut w, &h);
            w.append(payload);
        }
        w.into_bytes()
    }
}

// ─── Decoder ────────────────────────────────────────────────────────────────

pub struct MessageDecoder {
    pub encoded: bool,
    mailbox: bool,
    /// Message id → (type, mailbox key).
    ids: TwoGen<u64, (u64, u64)>,
    /// Message id → (frame, payload words) of its last payload.
    prev: TwoGen<u64, (u32, Vec<u32>)>,
}

impl MessageDecoder {
    /// A reliable-channel (`false`) or mailbox-channel (`true`) decoder:
    /// mailbox records carry a key after the type.
    pub fn new(mailbox: bool) -> Self {
        Self {
            encoded: false,
            mailbox,
            ids: TwoGen::new(ID_CAP, usize::MAX, |_| 0),
            prev: TwoGen::new(ID_CAP, BASE_BYTES_CAP, |(_, w)| w.len() * 4),
        }
    }

    /// Decode one frame body into `out`. On a malformed frame, `out` keeps
    /// the messages read before the fault.
    pub fn decode(&mut self, data: &[u8], out: &mut Vec<DecodedMessage>) {
        if data.is_empty() {
            return;
        }
        if self.encoded { self.decode_encoded(data, out) } else { self.decode_raw(data, out) }
    }

    fn decode_raw(&self, data: &[u8], out: &mut Vec<DecodedMessage>) {
        let word = |at: usize| data.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().expect("8 bytes")));
        let mut at = 0;
        while at + 16 <= data.len() {
            let Some(type_) = word(at) else { return };
            at += 8;
            let mailbox = if self.mailbox {
                let Some(k) = word(at) else { return };
                at += 8;
                k
            } else {
                RELIABLE_KEY
            };
            let Some(size) = word(at) else { return };
            at += 8;
            if size > (data.len() - at) as u64 {
                return;
            }
            let size = size as usize;
            out.push(DecodedMessage { size_bytes: size as u32, type_, mailbox, data: data[at..at + size].to_vec() });
            at = (at + size).div_ceil(8) * 8;
        }
    }

    fn decode_encoded(&mut self, data: &[u8], out: &mut Vec<DecodedMessage>) {
        let words: Vec<u64> = data
            .chunks(8)
            .map(|c| {
                let mut b = [0u8; 8];
                b[..c.len()].copy_from_slice(c);
                u64::from_le_bytes(b)
            })
            .collect();
        let mut r = BitReader { words: &words, pos: 0, end: words.len() * 64 };
        let h = Ranges::read(&mut r, self.mailbox);
        // Bounds from the frame's own size: at least a byte per message, and
        // at most 64 bytes out per byte in (deltas can grow).
        let max_msgs = (data.len()).clamp(1, 4_096);
        let max_total = data.len().saturating_mul(64).clamp(4_096, 1 << 20);
        if h.count as usize > max_msgs {
            return;
        }
        let mut total = 0usize;
        for _ in 0..h.count {
            let mut rec = Record::read(&mut r, &h);
            if rec.size as usize > MAX_MESSAGE_BYTES {
                return;
            }
            total = total.saturating_add(rec.size as usize);
            if total > max_total {
                return;
            }
            if !rec.fresh {
                let Some(&(t, k)) = self.ids.get(&rec.id) else {
                    r.skip(rec.bits as usize);
                    continue;
                };
                (rec.type_, rec.key) = (t, k);
            }
            self.ids.insert(rec.id, (rec.type_, rec.key));

            let n_words = (rec.size as usize).div_ceil(4);
            let mut words = match (self.prev.take(&rec.id), rec.relative) {
                // A relative payload needs this id's payload of the frame before.
                (Some((f, w)), true) if (f.wrapping_add(1) & 0x3f) == (rec.frame & 0x3f) => w,
                (_, true) => {
                    r.skip(rec.bits as usize);
                    continue;
                }
                (Some((_, mut w)), false) => {
                    w.clear();
                    w
                }
                (None, false) => Vec::new(),
            };
            words.resize(n_words, 0);

            let mut p = BitReader { words: r.words, pos: r.pos, end: r.end };
            let sub = rec.smode && !rec.xmode;
            let sub_or_xor = rec.smode && rec.xmode;
            let mut run = 0usize;
            for i in 0..n_words {
                let prev = words[i];
                if run > 0 {
                    run -= 1;
                    continue;
                }
                if rec.partial && !p.bit() {
                    // Unchanged: this word and a run of the ones after it.
                    let rest = n_words - i - 1;
                    let meta = if rest > 0 { width(width(rest as u64) as u64) } else { 0 };
                    let n = p.bits(meta) as usize;
                    run = p.bits(n) as usize;
                    continue;
                }
                words[i] = if sub || (sub_or_xor && p.bit()) {
                    let d = p.ranged(0, u64::from(rec.sub_off));
                    (i64::from(prev).wrapping_add(d as i64).wrapping_add(i64::from(rec.sub_min))) as u32
                } else {
                    read_word(&mut p, prev)
                };
            }

            let mut payload: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            payload.truncate(rec.size as usize);
            self.prev.insert(rec.id, (rec.frame, words));
            out.push(DecodedMessage { size_bytes: rec.size, type_: rec.type_, mailbox: rec.key, data: payload });
            r.skip(rec.bits as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(mailbox: bool, encoded: bool) {
        let mut enc = MessageEncoder::new(mailbox);
        let mut dec = MessageDecoder::new(mailbox);
        if encoded {
            enc.enable_ece();
            dec.encoded = true;
        }
        for frame in 0..4u8 {
            enc.begin_frame();
            let msgs: Vec<(u64, u64, Vec<u8>)> = (0..5u8)
                .map(|i| (0x1234_5678_9abc_def0 ^ u64::from(i), u64::from(i) * 7, (0..(i * 3 + frame)).map(|b| b.wrapping_mul(37) ^ i).collect()))
                .collect();
            for (t, k, d) in &msgs {
                enc.encode(*t, *k, d);
            }
            let body = enc.end_frame();
            let mut out = Vec::new();
            dec.decode(&body, &mut out);
            assert_eq!(out.len(), msgs.len());
            for (m, (t, k, d)) in out.iter().zip(&msgs) {
                assert_eq!((m.type_, &m.data), (*t, d));
                assert_eq!(m.mailbox, if mailbox { *k } else { RELIABLE_KEY });
            }
        }
    }

    #[test]
    fn raw_and_encoded_round_trip() {
        for mailbox in [false, true] {
            for encoded in [false, true] {
                round_trip(mailbox, encoded);
            }
        }
    }
}
