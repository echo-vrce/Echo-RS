//! The 64-bit header at the front of every decrypted transport packet.
//!
//! | bits  | field                                   |
//! |-------|-----------------------------------------|
//! | 0     | reliable channel                        |
//! | 1     | fragmented                              |
//! | 2-11  | frame number                            |
//! | 12-21 | reliable ack (next frame expected)      |
//! | 22-31 | mailbox ack                             |
//! | 32-39 | source id                               |
//! | 40-47 | destination id                          |
//! | 48    | body is codec-encoded                   |
//! | 53-63 | checksum                                |

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct PacketHeader(u64);

impl PacketHeader {
    pub fn new() -> Self {
        Self(0)
    }

    fn get(self, shift: u32, mask: u64) -> u64 {
        (self.0 >> shift) & mask
    }

    fn set(self, shift: u32, mask: u64, v: u64) -> Self {
        Self((self.0 & !(mask << shift)) | ((v & mask) << shift))
    }

    pub fn reliable(self) -> bool {
        self.get(0, 1) != 0
    }
    pub fn fragmented(self) -> bool {
        self.get(1, 1) != 0
    }
    pub fn frame(self) -> u32 {
        self.get(2, 0x3ff) as u32
    }
    pub fn reliack(self) -> u32 {
        self.get(12, 0x3ff) as u32
    }
    pub fn mailack(self) -> u32 {
        self.get(22, 0x3ff) as u32
    }
    pub fn srcid(self) -> u8 {
        self.get(32, 0xff) as u8
    }
    pub fn encoded(self) -> bool {
        self.get(48, 1) != 0
    }

    pub fn with_reliable(self, v: bool) -> Self {
        self.set(0, 1, u64::from(v))
    }
    pub fn with_fragmented(self, v: bool) -> Self {
        self.set(1, 1, u64::from(v))
    }
    pub fn with_frame(self, v: u32) -> Self {
        self.set(2, 0x3ff, u64::from(v))
    }
    pub fn with_reliack(self, v: u32) -> Self {
        self.set(12, 0x3ff, u64::from(v))
    }
    pub fn with_mailack(self, v: u32) -> Self {
        self.set(22, 0x3ff, u64::from(v))
    }
    pub fn with_srcid(self, v: u8) -> Self {
        self.set(32, 0xff, u64::from(v))
    }
    pub fn with_dstid(self, v: u8) -> Self {
        self.set(40, 0xff, u64::from(v))
    }
    pub fn with_encoded(self, v: bool) -> Self {
        self.set(48, 1, u64::from(v))
    }
    pub fn with_checksum(self, v: u16) -> Self {
        self.set(53, 0x7ff, u64::from(v))
    }

    pub fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }
    pub fn from_le_bytes(b: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(b))
    }
}

/// The 11-bit packet checksum: the header's 16-bit words (checksum cleared,
/// top word cut to 5 bits) XORed with every body byte, sign-extended, put in
/// the low byte at even offsets and the high byte at odd ones.
pub fn checksum(header: PacketHeader, body: &[u8]) -> u16 {
    let h = header.with_checksum(0).0;
    let mut cs = (h as u16) ^ ((h >> 16) as u16) ^ ((h >> 32) as u16) ^ ((h >> 48) as u16 & 0x1f);
    for (i, &b) in body.iter().enumerate() {
        let wide = i16::from(b as i8) as u16;
        cs ^= if i & 1 == 0 { wide } else { wide << 8 };
    }
    cs & 0x7ff
}
