//! Packet crypto of the game's UDP transport.
//!
//! A datagram is `[HMAC-SHA512 prefix][sequence u64 LE][payload]`. The MAC
//! covers everything after itself. The payload is AES-256-CBC with PKCS#7
//! padding when encryption is on. The IV is never sent: both ends draw one
//! from a Keccak-f[1600] generator seeded with the session's random key, one
//! IV per sequence number.

use aes::Aes256;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use hmac::{Hmac, Mac};
use sha2::Sha512;

const SEQ_LEN: usize = 8;

/// The packet-encoding flags a session announces (`*_encoder_flags`).
#[derive(Debug, Clone, Copy)]
pub struct WireConfig {
    pub encrypt: bool,
    pub mac: bool,
    /// Bytes of the HMAC-SHA512 digest kept at the front of each packet.
    pub mac_len: usize,
}

impl WireConfig {
    /// Bits: 0 encryption, 1 MAC, 2..14 digest size (the rest are key sizes
    /// and the PBKDF2 count, which the session keys already fix).
    pub const fn from_flags(flags: u64) -> Self {
        Self { encrypt: flags & 1 != 0, mac: (flags >> 1) & 1 != 0, mac_len: ((flags >> 2) & 0xfff) as usize }
    }

    /// Length of the MAC prefix on the wire.
    const fn prefix(&self) -> usize {
        if self.mac { self.mac_len } else { 0 }
    }
}

pub struct SessionKeys {
    pub aes: [u8; 32],
    pub mac: [u8; 32],
}

impl SessionKeys {
    pub fn new(aes: [u8; 32], mac: [u8; 32]) -> Self {
        Self { aes, mac }
    }
}

fn hmac512(key: &[u8; 32], data: &[u8]) -> [u8; 64] {
    let mut m = <Hmac<Sha512> as Mac>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(data);
    m.finalize().into_bytes().into()
}

/// A packet whose MAC checked out (or that carries none).
#[derive(Clone, Copy)]
pub struct Authenticated<'a> {
    packet: &'a [u8],
    prefix: usize,
    encrypt: bool,
}

impl Authenticated<'_> {
    pub fn sequence(&self) -> u64 {
        u64::from_le_bytes(self.packet[self.prefix..self.prefix + SEQ_LEN].try_into().expect("length checked"))
    }
}

/// Check a packet's MAC. `None` if it is too short or the MAC is wrong.
pub fn authenticate<'a>(wire: &WireConfig, keys: &SessionKeys, packet: &'a [u8]) -> Option<Authenticated<'a>> {
    let prefix = wire.prefix();
    if packet.len() < prefix + SEQ_LEN || prefix > 64 {
        return None;
    }
    if prefix > 0 {
        let (mac, rest) = packet.split_at(prefix);
        let want = hmac512(&keys.mac, rest);
        // Constant time over the prefix.
        if want[..prefix].iter().zip(mac).fold(0u8, |acc, (a, b)| acc | (a ^ b)) != 0 {
            return None;
        }
    }
    Some(Authenticated { packet, prefix, encrypt: wire.encrypt })
}

/// Decrypt an authenticated packet's payload into `out`.
pub fn decrypt(auth: Authenticated<'_>, keys: &SessionKeys, iv: &[u8; 16], out: &mut Vec<u8>) -> Option<()> {
    let payload = &auth.packet[auth.prefix + SEQ_LEN..];
    out.clear();
    out.extend_from_slice(payload);
    if auth.encrypt && !payload.is_empty() {
        let dec = cbc::Decryptor::<Aes256>::new(&keys.aes.into(), iv.into());
        let len = dec.decrypt_padded_mut::<Pkcs7>(out).ok()?.len();
        out.truncate(len);
    }
    Some(())
}

/// Build one datagram: MAC prefix, sequence, (encrypted) payload.
pub fn seal(wire: &WireConfig, keys: &SessionKeys, iv: &[u8; 16], seq: u64, plain: &[u8]) -> Vec<u8> {
    let prefix = wire.prefix().min(64);
    let body = if wire.encrypt { (plain.len() / 16 + 1) * 16 } else { plain.len() };
    let start = prefix + SEQ_LEN;
    let mut out = vec![0u8; start + body];
    out[prefix..start].copy_from_slice(&seq.to_le_bytes());
    out[start..start + plain.len()].copy_from_slice(plain);
    if wire.encrypt {
        let enc = cbc::Encryptor::<Aes256>::new(&keys.aes.into(), iv.into());
        if enc.encrypt_padded_mut::<Pkcs7>(&mut out[start..], plain.len()).is_err() {
            return Vec::new();
        }
    }
    if prefix > 0 {
        let mac = hmac512(&keys.mac, &out[prefix..]);
        out[..prefix].copy_from_slice(&mac[..prefix]);
    }
    out
}

// ─── IV generator ───────────────────────────────────────────────────────────

const ROUND_CONSTANTS: [u64; 24] = [
    0x0000_0000_0000_0001,
    0x0000_0000_0000_8082,
    0x8000_0000_0000_808a,
    0x8000_0000_8000_8000,
    0x0000_0000_0000_808b,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8009,
    0x0000_0000_0000_008a,
    0x0000_0000_0000_0088,
    0x0000_0000_8000_8009,
    0x0000_0000_8000_000a,
    0x0000_0000_8000_808b,
    0x8000_0000_0000_008b,
    0x8000_0000_0000_8089,
    0x8000_0000_0000_8003,
    0x8000_0000_0000_8002,
    0x8000_0000_0000_0080,
    0x0000_0000_0000_800a,
    0x8000_0000_8000_000a,
    0x8000_0000_8000_8081,
    0x8000_0000_0000_8080,
    0x0000_0000_8000_0001,
    0x8000_0000_8000_8008,
];

/// Rotation of lane `x + 5y`.
const ROTATIONS: [u32; 25] = [0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56, 14];

/// The Keccak-f[1600] permutation.
fn keccak_f(a: &mut [u64; 25]) {
    for rc in ROUND_CONSTANTS {
        // θ
        let mut c = [0u64; 5];
        for x in 0..5 {
            c[x] = a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20];
        }
        for x in 0..5 {
            let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            for y in 0..5 {
                a[x + 5 * y] ^= d;
            }
        }
        // ρ and π
        let mut b = [0u64; 25];
        for x in 0..5 {
            for y in 0..5 {
                b[y + 5 * ((2 * x + 3 * y) % 5)] = a[x + 5 * y].rotate_left(ROTATIONS[x + 5 * y]);
            }
        }
        // χ
        for y in 0..5 {
            for x in 0..5 {
                a[x + 5 * y] = b[x + 5 * y] ^ (!b[(x + 1) % 5 + 5 * y] & b[(x + 2) % 5 + 5 * y]);
            }
        }
        // ι
        a[0] ^= rc;
    }
}

/// The game's IV generator: a Keccak state filled by a fixed xorshift, then
/// absorbing the 32-byte random key one word per permutation.
pub struct IvGenerator {
    state: [u64; 25],
}

impl IvGenerator {
    pub fn new(seed: &[u8; 32]) -> Self {
        let mut g = Self { state: [0; 25] };
        g.fill();
        for w in seed.chunks_exact(8) {
            g.step(u64::from_le_bytes(w.try_into().expect("8 bytes")));
        }
        g
    }

    /// The fixed starting state: an xorshift64* ring of 16, then a
    /// xorshift1024-style walk over it for the 25 lanes.
    fn fill(&mut self) {
        let mut ring = [0u64; 16];
        let mut x: u64 = 0x3130_fc21_c865_52e6;
        for r in &mut ring {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *r = x.wrapping_mul(0x2545_f491_4f6c_dd1d);
        }
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        let mut p = (x.wrapping_mul(3u64.wrapping_neg()) & 0xf) as usize;
        let mut s = ring[p];
        for lane in &mut self.state {
            p = (p + 1) & 0xf;
            let mut t = ring[p];
            t ^= t << 31;
            ring[p] = (((s >> 19) ^ t) >> 11) ^ t ^ s;
            s = ring[p];
            *lane = s.wrapping_mul(0x1066_89d4_5497_fdb5);
        }
        self.step(0);
    }

    fn step(&mut self, input: u64) -> u64 {
        self.state[0] ^= input;
        keccak_f(&mut self.state);
        self.state[0]
    }

    /// The next 16-byte IV (two words).
    pub fn next_iv(&mut self) -> [u8; 16] {
        let mut iv = [0u8; 16];
        iv[..8].copy_from_slice(&self.step(0).to_le_bytes());
        iv[8..].copy_from_slice(&self.step(0).to_le_bytes());
        iv
    }
}
