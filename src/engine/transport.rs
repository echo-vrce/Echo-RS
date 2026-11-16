//! Per-peer UDP transport: packet crypto, the reliable and mailbox channels,
//! acknowledgement, retransmission and fragmentation.
//!
//! Wire layout of one datagram:
//! `[MAC][seq u64][AES-256-CBC([header u64][body])]`. The IV is never sent:
//! both ends draw it from a Keccak PRNG seeded with the session's random key,
//! one IV per sequence number (including numbers the sender skipped).
//!
//! Only the primitives come from `nevr-crypto` (AES/HMAC/Keccak) and
//! `rad-protocol` (message codec, header bits, checksum).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use nevr_crypto::aes_cbc::{self, SessionKeys, WireConfig};
use nevr_crypto::keccak::KeccakPrng;
use rad_protocol::codec::decoder::{DecodedMessage, MessageDecoder};
use rad_protocol::codec::encoder::MessageEncoder;
use rad_protocol::header::{PacketHeader, compute_checksum};

/// Frame numbers wrap on a 1023-entry ring (0x3ff is "none").
pub const FRAME_RING: u16 = 1023;
/// Unacked reliable frames allowed in flight.
const WINDOW: u16 = 511;
/// Client receive buffer is 1264 bytes; leave room for MAC(32), seq, padding,
/// header and the fragment index words.
pub const MAX_CHUNK: usize = 1264 - 32 - 8 - 16 - 8 - 4;
/// Our broadcaster id in outbound headers.
const OUR_SRCID: u8 = 0xef;
/// Reliable dst word.
pub const RELIABLE_DST: u64 = u64::MAX;
/// A reliable packet unacked this long means the peer is gone.
const RTT_SAMPLES: usize = 32;
const ACK_TIMEOUT: Duration = Duration::from_secs(15);
/// Inbound IVs remembered for reordered packets.
const IV_MEMORY: usize = 512;
/// Largest forward sequence jump accepted (bounds PRNG work per packet).
const MAX_SEQ_JUMP: u64 = 8192;

pub struct SessionCrypto {
    pub out_flags: u64,
    pub out_aes: [u8; 32],
    pub out_mac: [u8; 32],
    pub out_rnd: [u8; 32],
    pub out_seq: u64,
    pub in_flags: u64,
    pub in_aes: [u8; 32],
    pub in_mac: [u8; 32],
    pub in_rnd: [u8; 32],
    pub in_seq: u64,
}

struct Outbound {
    wire: WireConfig,
    keys: SessionKeys,
    prng: KeccakPrng,
    seq: u64,
}

struct Inbound {
    wire: WireConfig,
    keys: SessionKeys,
    prng: KeccakPrng,
    init_seq: u64,
    /// PRNG index of the next IV to draw.
    drawn: u64,
    ivs: VecDeque<(u64, [u8; 16])>,
}

impl Inbound {
    fn iv_for(&mut self, seq: u64) -> Option<[u8; 16]> {
        let idx = seq.wrapping_sub(self.init_seq);
        if idx >= self.drawn {
            if idx - self.drawn > MAX_SEQ_JUMP {
                return None;
            }
            while self.drawn <= idx {
                let iv = self.prng.next_iv();
                self.ivs.push_back((self.drawn, iv));
                if self.ivs.len() > IV_MEMORY {
                    self.ivs.pop_front();
                }
                self.drawn += 1;
            }
        }
        self.ivs.iter().rev().find(|(i, _)| *i == idx).map(|(_, iv)| *iv)
    }
}

/// Check whether `packet` was MACed with these inbound keys (peer claim).
pub fn mac_matches(flags: u64, mac: &[u8; 32], packet: &[u8]) -> bool {
    let wire = WireConfig::from_flags(flags);
    let keys = SessionKeys::new([0; 32], *mac);
    aes_cbc::authenticate_packet(&wire, &keys, packet).is_some()
}

struct Retained {
    frame: u16,
    plain: Vec<u8>,
    sent: Instant,
    first: Instant,
}

#[derive(Default)]
pub struct Received {
    /// Messages from the reliable channel, in order.
    pub reliable: Vec<DecodedMessage>,
    /// Messages from the mailbox channel.
    pub mailbox: Vec<DecodedMessage>,
}

pub struct Transport {
    out: Outbound,
    inb: Inbound,
    rel_enc: MessageEncoder,
    mb_enc: MessageEncoder,
    rel_encoded: bool,
    mb_encoded: bool,
    rel_ece_pending: bool,
    mb_ece_pending: bool,
    rel_dec: MessageDecoder,
    mb_dec: MessageDecoder,
    /// Next outbound frame per channel.
    rel_next: u16,
    mb_next: u16,
    /// Next expected inbound frame per channel (= our reliack / mailack).
    rel_expect: u16,
    mb_expect: u16,
    /// Peer's ack of our reliable frames (next frame it expects).
    rel_acked: u16,
    retained: VecDeque<Retained>,
    /// When each outbound mailbox frame left, for round-trip timing.
    mb_sent: Vec<Option<Instant>>,
    /// Peer's last mailack (next mailbox frame it expects from us).
    last_mailack: u16,
    /// Round trips (µs): our mailbox frame out, the peer's mailack of it back.
    rtt_samples: VecDeque<u32>,
    rtt_fresh: bool,
    /// Gaps (µs) between the peer's mailbox frames: its send cadence.
    mb_gaps: VecDeque<u32>,
    last_mb_in: Option<Instant>,
    parked: BTreeMap<u16, Vec<u8>>,
    fragments: HashMap<(bool, u16), (u16, Vec<Option<Vec<u8>>>)>,
    dstid: u8,
    pub queued_reliable: Vec<(u64, Vec<u8>)>,
    pub queued_mailbox: Vec<(u64, u64, Vec<u8>)>,
    ack_owed: bool,
    /// Smoothed reliable RTT for the resend timer.
    srtt: Duration,
    pub packets_out: u64,
    pub packets_in: u64,
}

fn ring_add(f: u16, n: u16) -> u16 {
    ((u32::from(f) + u32::from(n)) % u32::from(FRAME_RING)) as u16
}

/// Distance from `from` forward to `to` on the frame ring.
fn ring_ahead(from: u16, to: u16) -> u16 {
    ((u32::from(to) + u32::from(FRAME_RING) - u32::from(from)) % u32::from(FRAME_RING)) as u16
}

impl Transport {
    pub fn new(c: &SessionCrypto) -> Self {
        let out = Outbound {
            wire: WireConfig::from_flags(c.out_flags),
            keys: SessionKeys::new(c.out_aes, c.out_mac),
            prng: KeccakPrng::new(&c.out_rnd),
            seq: c.out_seq,
        };
        let inb = Inbound {
            wire: WireConfig::from_flags(c.in_flags),
            keys: SessionKeys::new(c.in_aes, c.in_mac),
            prng: KeccakPrng::new(&c.in_rnd),
            init_seq: c.in_seq,
            drawn: 0,
            ivs: VecDeque::with_capacity(64),
        };
        Self {
            out,
            inb,
            rel_enc: MessageEncoder::new(),
            mb_enc: MessageEncoder::with_mailbox(true),
            rel_encoded: false,
            mb_encoded: false,
            rel_ece_pending: false,
            mb_ece_pending: false,
            rel_dec: MessageDecoder::new(),
            mb_dec: {
                // Mailbox records carry a key after the type; without this
                // every record after the first in a frame is misread.
                let mut d = MessageDecoder::new();
                d.is_mailbox = true;
                d
            },
            rel_next: 0,
            mb_next: 0,
            mb_sent: vec![None; usize::from(FRAME_RING)],
            last_mailack: u16::MAX,
            rtt_samples: VecDeque::with_capacity(RTT_SAMPLES),
            rtt_fresh: false,
            mb_gaps: VecDeque::with_capacity(RTT_SAMPLES),
            last_mb_in: None,
            rel_expect: 0,
            mb_expect: 0,
            rel_acked: 0,
            retained: VecDeque::new(),
            parked: BTreeMap::new(),
            fragments: HashMap::new(),
            dstid: 0xff,
            queued_reliable: Vec::new(),
            queued_mailbox: Vec::new(),
            ack_owed: false,
            srtt: Duration::from_millis(30),
            packets_out: 0,
            packets_in: 0,
        }
    }

    /// Decrypt and process one datagram. Returns the decoded messages, or
    /// `None` if the packet is not ours.
    pub fn receive(&mut self, packet: &[u8]) -> Option<Received> {
        let auth = aes_cbc::authenticate_packet(&self.inb.wire, &self.inb.keys, packet)?;
        let seq = auth.sequence();
        let iv = self.inb.iv_for(seq)?;
        let mut plain = Vec::with_capacity(packet.len());
        aes_cbc::decrypt_authenticated_into(auth, &self.inb.keys, &iv, &mut plain).ok()?;
        if plain.len() < 8 {
            return None;
        }
        self.packets_in += 1;
        let hdr = PacketHeader::from_le_bytes(plain[..8].try_into().ok()?);
        if hdr.srcid() != 0xff {
            self.dstid = hdr.srcid();
        }
        self.on_ack(hdr.reliack() as u16);
        self.on_mailack(hdr.mailack() as u16);
        let mut out = Received::default();
        if plain.len() == 8 {
            return Some(out); // header-only ack
        }
        let reliable = hdr.reliable();
        let frame = hdr.frame() as u16;
        let body = if hdr.fragmented() {
            if plain.len() < 12 {
                return Some(out);
            }
            let idx = u16::from_le_bytes([plain[8], plain[9]]);
            let last = u16::from_le_bytes([plain[10], plain[11]]);
            if last > 64 || idx > last {
                return Some(out);
            }
            let entry = self.fragments.entry((reliable, frame)).or_insert_with(|| (last, vec![None; usize::from(last) + 1]));
            if entry.0 != last {
                *entry = (last, vec![None; usize::from(last) + 1]);
            }
            entry.1[usize::from(idx)] = Some(plain[12..].to_vec());
            if entry.1.iter().any(Option::is_none) {
                if reliable {
                    self.ack_owed = true;
                }
                return Some(out);
            }
            let (_, parts) = self.fragments.remove(&(reliable, frame))?;
            let mut whole = plain[..8].to_vec();
            for p in parts.into_iter().flatten() {
                whole.extend_from_slice(&p);
            }
            if self.fragments.len() > 64 {
                self.fragments.clear();
            }
            whole
        } else {
            plain
        };
        if reliable {
            self.ack_owed = true;
            if frame == self.rel_expect {
                self.decode_into(true, &body, &mut out.reliable);
                self.rel_expect = ring_add(self.rel_expect, 1);
                while let Some(next) = self.parked.remove(&self.rel_expect) {
                    self.decode_into(true, &next, &mut out.reliable);
                    self.rel_expect = ring_add(self.rel_expect, 1);
                }
            } else if ring_ahead(self.rel_expect, frame) < WINDOW {
                if self.parked.len() < usize::from(WINDOW) {
                    self.parked.insert(frame, body);
                }
            } // else: a duplicate of a frame already delivered; the ack covers it
        } else {
            // Mailbox: latest-wins; never stall on a lost frame.
            if frame == self.mb_expect || ring_ahead(self.mb_expect, frame) < WINDOW {
                self.decode_into(false, &body, &mut out.mailbox);
                self.mb_expect = ring_add(frame, 1);
                let now = Instant::now();
                if let Some(prev) = self.last_mb_in.replace(now) {
                    if self.mb_gaps.len() >= RTT_SAMPLES {
                        self.mb_gaps.pop_front();
                    }
                    self.mb_gaps.push_back(now.duration_since(prev).as_micros() as u32);
                }
            }
        }
        Some(out)
    }

    fn decode_into(&mut self, reliable: bool, frame: &[u8], out: &mut Vec<DecodedMessage>) {
        let hdr = PacketHeader::from_le_bytes(frame[..8].try_into().unwrap_or([0; 8]));
        let dec = if reliable { &mut self.rel_dec } else { &mut self.mb_dec };
        if hdr.encoded() {
            dec.enable_encoding();
        }
        let raw_mode = !dec.encoding_enabled;
        dec.begin_frame();
        dec.set_input(&frame[8..]);
        let mut scratch = Vec::new();
        if dec.decode_input_into(raw_mode, &mut scratch).is_ok() || !scratch.is_empty() {
            for m in scratch {
                if reliable && m.type_ == crate::engine::msg::RELIABLE_BATCH {
                    unpack_batch(&m.data, out);
                } else {
                    out.push(m);
                }
            }
        }
    }

    /// A mailack that moved times the frame just before it: a real network
    /// round trip (plus at most one client frame), with nothing added.
    fn on_mailack(&mut self, ack: u16) {
        if ack >= FRAME_RING || ack == self.last_mailack {
            return;
        }
        self.last_mailack = ack;
        let f = usize::from((ack + FRAME_RING - 1) % FRAME_RING);
        if let Some(sent) = self.mb_sent[f].take() {
            let rtt = sent.elapsed();
            if rtt < Duration::from_secs(2) {
                if self.rtt_samples.len() >= RTT_SAMPLES {
                    self.rtt_samples.pop_front();
                }
                self.rtt_samples.push_back(rtt.as_micros() as u32);
                self.rtt_fresh = true;
            }
        }
    }

    /// Network round trip in µs: the minimum of recent samples, which strips
    /// the peer's send-alignment jitter.
    ///
    /// A client only acks on its next outgoing frame, so every sample also
    /// holds up to one of the client's own frames (13.9 ms at 72 Hz). That
    /// hold is the client's render cadence, not network distance, so one send
    /// interval (the median gap between its mailbox frames) is taken off.
    pub fn rtt_us(&self) -> Option<u32> {
        let min = self.rtt_samples.iter().copied().min()?;
        Some(min.saturating_sub(self.client_frame_us()))
    }

    /// The peer's send interval (µs): median gap between its mailbox frames.
    pub fn client_frame_us(&self) -> u32 {
        if self.mb_gaps.len() < 8 {
            return 0;
        }
        let mut g: Vec<u32> = self.mb_gaps.iter().copied().collect();
        g.sort_unstable();
        g[g.len() / 2]
    }

    pub fn raw_rtt_us(&self) -> Option<u32> {
        self.rtt_samples.iter().copied().min()
    }

    pub fn rtt_sample_count(&self) -> usize {
        self.rtt_samples.len()
    }

    /// True once per new round-trip sample.
    pub fn take_rtt_fresh(&mut self) -> bool {
        std::mem::take(&mut self.rtt_fresh)
    }

    fn on_ack(&mut self, ack: u16) {
        if ack >= FRAME_RING {
            return;
        }
        // `ack` is the next frame the peer expects; everything before it is in.
        let outstanding = ring_ahead(self.rel_acked, self.rel_next);
        let advance = ring_ahead(self.rel_acked, ack);
        if advance == 0 || advance > outstanding {
            return;
        }
        let now = Instant::now();
        while let Some(r) = self.retained.front() {
            if ring_ahead(self.rel_acked, r.frame) < advance {
                let r = self.retained.pop_front().expect("front exists");
                if r.sent == r.first {
                    let sample = now.duration_since(r.sent);
                    self.srtt = (self.srtt * 7 + sample) / 8;
                }
            } else {
                break;
            }
        }
        self.rel_acked = ack;
    }

    pub fn queue_reliable(&mut self, ty: u64, data: &[u8]) {
        self.queued_reliable.push((ty, data.to_vec()));
    }

    pub fn queue_mailbox(&mut self, ty: u64, key: u64, data: &[u8]) {
        self.queued_mailbox.push((ty, key, data.to_vec()));
    }

    /// Seconds the oldest unacked reliable frame has been waiting.
    pub fn stalled(&self) -> bool {
        self.retained.front().is_some_and(|r| r.first.elapsed() > ACK_TIMEOUT)
    }

    fn header(&self, reliable: bool, fragmented: bool, frame: u16, encoded: bool) -> PacketHeader {
        PacketHeader::new()
            .with_reliable(reliable)
            .with_fragmented(fragmented)
            .with_frame(u32::from(frame))
            .with_reliack(u32::from(self.rel_expect))
            .with_mailack(u32::from(self.mb_expect))
            .with_srcid(OUR_SRCID)
            .with_dstid(self.dstid)
            .with_encoded(encoded)
    }

    fn stamp(plain: &mut [u8], hdr: PacketHeader) {
        let cs = compute_checksum(hdr, &plain[8..]);
        plain[..8].copy_from_slice(&hdr.with_checksum(cs).to_le_bytes());
    }

    fn seal(&mut self, plain: &[u8], out: &mut Vec<Vec<u8>>) {
        let iv = self.out.prng.next_iv();
        let seq = self.out.seq;
        self.out.seq = self.out.seq.wrapping_add(1);
        out.push(aes_cbc::encrypt_packet_v2(&self.out.wire, &self.out.keys, &iv, seq, plain));
        self.packets_out += 1;
    }

    /// Build every datagram owed right now: resends, the queued reliable and
    /// mailbox frames, and a bare ack if nothing else carries it.
    pub fn flush(&mut self, out: &mut Vec<Vec<u8>>) {
        self.flush_some(out, true);
    }

    /// Between server frames: resends and reliable frames only. Mailbox
    /// state and bare acks wait for the next frame, so each peer gets about
    /// one packet per frame (as native does) rather than one per packet any
    /// other player sent: with 12 players that was ~1,300 packets a second
    /// to every client, and everyone stuttered.
    pub fn flush_reliable(&mut self, out: &mut Vec<Vec<u8>>) {
        self.flush_some(out, false);
    }

    fn flush_some(&mut self, out: &mut Vec<Vec<u8>>, frame: bool) {
        let start = out.len();
        let now = Instant::now();
        // Resends first, restamped with current acks.
        // Resend after 1.5 round trips. The round trip is the larger of the
        // reliable channel's own (only never-resent frames are timed) and the
        // mailbox acks'. A 60 ms ceiling here resent every frame two or three
        // times to anyone with a ping over 60 ms, which was most players.
        let measured = self.rtt_samples.iter().copied().min().map_or(Duration::ZERO, |us| Duration::from_micros(u64::from(us)));
        let rtt = self.srtt.max(measured);
        let rto = (rtt * 3 / 2 + Duration::from_millis(10)).clamp(Duration::from_millis(40), Duration::from_millis(500));
        let mut resend = Vec::new();
        for (i, r) in self.retained.iter().enumerate() {
            if now.duration_since(r.sent) >= rto {
                resend.push(i);
            }
        }
        for i in resend {
            let mut plain = std::mem::take(&mut self.retained[i].plain);
            let old = PacketHeader::from_le_bytes(plain[..8].try_into().expect("8 bytes"));
            let hdr = old
                .with_checksum(0)
                .with_reliack(u32::from(self.rel_expect))
                .with_mailack(u32::from(self.mb_expect))
                .with_dstid(self.dstid);
            Self::stamp(&mut plain, hdr);
            self.seal(&plain, out);
            let r = &mut self.retained[i];
            r.plain = plain;
            r.sent = now;
        }

        // Reliable: one frame per call, up to 256 messages.
        while !self.queued_reliable.is_empty() && ring_ahead(self.rel_acked, self.rel_next) < WINDOW {
            if self.rel_ece_pending {
                self.rel_encoded = true;
                self.rel_ece_pending = false;
            }
            self.rel_enc.begin_frame();
            let mut ece = false;
            let take = self.queued_reliable.len().min(256);
            for (ty, data) in self.queued_reliable.drain(..take) {
                ece |= ty == crate::engine::msg::ENABLE_COMPRESSION;
                self.rel_enc.encode(ty, RELIABLE_DST, &data);
            }
            let body = self.rel_enc.end_frame();
            let frame = self.rel_next;
            self.rel_next = ring_add(self.rel_next, 1);
            let encoded = self.rel_encoded;
            for plain in self.packetize(true, frame, encoded, &body) {
                self.seal(&plain, out);
                self.retained.push_back(Retained { frame, plain, sent: now, first: now });
            }
            if ece {
                // The frame announcing compression goes raw; the next is encoded.
                self.rel_enc.enable_ece();
                self.mb_enc.enable_ece();
                self.rel_ece_pending = true;
                self.mb_ece_pending = true;
            }
        }

        // Mailbox: unreliable, latest state wins on the receiver.
        while frame && !self.queued_mailbox.is_empty() {
            if self.mb_ece_pending {
                self.mb_encoded = true;
                self.mb_ece_pending = false;
            }
            self.mb_enc.begin_frame();
            // Whole messages up to one datagram's worth: a fragmented
            // mailbox frame is lost whole if any piece is.
            let mut size = 0;
            let take = self
                .queued_mailbox
                .iter()
                .take(256)
                .take_while(|(_, _, d)| {
                    size += d.len() + 24;
                    size <= MAX_CHUNK
                })
                .count()
                .max(1);
            for (ty, key, data) in self.queued_mailbox.drain(..take) {
                self.mb_enc.encode(ty, key, &data);
            }
            let body = self.mb_enc.end_frame();
            let frame = self.mb_next;
            self.mb_next = ring_add(self.mb_next, 1);
            self.mb_sent[usize::from(frame)] = Some(now);
            let encoded = self.mb_encoded;
            for plain in self.packetize(false, frame, encoded, &body) {
                self.seal(&plain, out);
            }
        }

        if out.len() > start {
            self.ack_owed = false;
        } else if self.ack_owed && frame {
            self.ack_owed = false;
            let mut plain = vec![0u8; 8];
            let hdr = self.header(false, false, self.mb_next, self.mb_encoded);
            Self::stamp(&mut plain, hdr);
            self.seal(&plain, out);
        }
    }

    fn packetize(&self, reliable: bool, frame: u16, encoded: bool, body: &[u8]) -> Vec<Vec<u8>> {
        if body.len() <= MAX_CHUNK {
            let mut plain = Vec::with_capacity(8 + body.len());
            plain.extend_from_slice(&[0; 8]);
            plain.extend_from_slice(body);
            let hdr = self.header(reliable, false, frame, encoded);
            Self::stamp(&mut plain, hdr);
            return vec![plain];
        }
        let chunks: Vec<&[u8]> = body.chunks(MAX_CHUNK).collect();
        let last = (chunks.len() - 1) as u16;
        chunks
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut plain = Vec::with_capacity(12 + c.len());
                plain.extend_from_slice(&[0; 8]);
                plain.extend_from_slice(&(i as u16).to_le_bytes());
                plain.extend_from_slice(&last.to_le_bytes());
                plain.extend_from_slice(c);
                let hdr = self.header(reliable, true, frame, encoded);
                Self::stamp(&mut plain, hdr);
                plain
            })
            .collect()
    }
}

/// `SBroadcasterReliableBatchEvent`: a marker byte, then
/// `[type u64][len u32][data]` entries.
pub fn reliable_batch(entries: &[(u64, &[u8])]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + entries.iter().map(|(_, d)| 12 + d.len()).sum::<usize>());
    out.push(0xcc);
    for (ty, data) in entries {
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn unpack_batch(data: &[u8], out: &mut Vec<DecodedMessage>) {
    let mut i = 1;
    while i + 12 <= data.len() {
        let ty = u64::from_le_bytes(data[i..i + 8].try_into().expect("8 bytes"));
        let len = u32::from_le_bytes(data[i + 8..i + 12].try_into().expect("4 bytes")) as usize;
        i += 12;
        if i + len > data.len() {
            break;
        }
        out.push(DecodedMessage { size_bytes: len as u32, type_: ty, mailbox: RELIABLE_DST, data: data[i..i + len].to_vec() });
        i += len;
    }
}
