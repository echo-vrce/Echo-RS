//! The game's two name hashes.
//!
//! * [`symbol`]: a CRC-64 style hash (MSB first, lower-cased input), used for
//!   symbols in JSON, ServerDB frames and the login/config services.
//! * [`mat_sym`]: the GF(2) matrix hash the game uses for network message
//!   type ids (`CMatSym::Hash` in echovr.exe).
//!
//! Both are `const fn`, so message ids can be compile-time constants.

/// The CRC polynomial of [`symbol`].
const SYMBOL_POLY: u64 = 0x2b59_2653_5897_936a;

/// `SYMBOL_TABLE[b]` is the carry-less product `b * SYMBOL_POLY`, truncated to
/// 64 bits (that is the game's table: no reduction).
static SYMBOL_TABLE: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = 0u64;
        let mut b = 0;
        while b < 8 {
            if (i >> b) & 1 != 0 {
                v ^= SYMBOL_POLY << b;
            }
            b += 1;
        }
        t[i] = v;
        i += 1;
    }
    t
};

/// Continue a [`symbol`] hash from `value` over `name` (ASCII case-folded).
pub const fn symbol_from(name: &str, mut value: u64) -> u64 {
    let bytes = name.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i].to_ascii_lowercase();
        value = SYMBOL_TABLE[(value >> 56) as usize] ^ (c as u64) ^ (value << 8);
        i += 1;
    }
    value
}

/// The game's symbol hash of `name` (starts from all ones).
pub const fn symbol(name: &str) -> u64 {
    symbol_from(name, u64::MAX)
}

/// `STRETCH[b]`: every set bit of `b` widened to a whole `0xff` byte.
static STRETCH: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut v = 0u64;
        let mut b = 0;
        while b < 8 {
            if (i >> b) & 1 != 0 {
                v |= 0xff << (8 * b);
            }
            b += 1;
        }
        t[i] = v;
        i += 1;
    }
    t
};

/// Per-character matrices of [`mat_sym`], as the game's binary holds them.
static MAT_SEEDS: [u64; 256] = [
    0x2bdda74beaacabc3, 0x8869c65feb52db80, 0x5723c9f6cb908143, 0x8f32573790804cf3,
    0xff1ba8836937ac9a, 0x7c20ece554d76d16, 0x8aa7a9ffc26fb5cf, 0x09528283645e5dce,
    0x6787c29065e52f19, 0x90940ad579ec8f19, 0x76e9c71b2825a262, 0x9b660273adfa8368,
    0x0d911813e4519293, 0x35d55bb82066a822, 0x0b39689134c9a6b9, 0x21e83e1af57281d8,
    0x2b3d6d235326e186, 0x3f2158c687d0afc2, 0xbe8b2d251cc01e65, 0x8a376215c574365f,
    0x15d8d74207be1e9f, 0xd7b19efb7acce172, 0xb38f4e35f6f7a91e, 0x853fb06d889e2a48,
    0xef4f0c88be562058, 0x14e0b6e48d0c0f16, 0xccdb5bba150c1288, 0x71b40cd4946e0b37,
    0x50f9cba1770a2b63, 0x3fad46d8b8069923, 0x81a77f3b7678b640, 0xcb8af474d7262810,
    0xf976a95598b999d2, 0xfda3f52463b46c6e, 0xd64815138c09f229, 0xb3917ca22f79ceee,
    0x5c34055f26a1e404, 0x1ef1e6511d3f69a6, 0x1cb54571b9d18ab6, 0xe2bed505ba1caf02,
    0xe9b31894d4e6b04b, 0x3cfda4e0033eaf63, 0xfed841d74060e331, 0x327cdc166506e07f,
    0xe3066b4b6a3be136, 0x4cb1bc3021704523, 0x8c9333f52f505b76, 0xfc18adb105f7c9b8,
    0x2327982451117f85, 0x613fcaecc73a627f, 0xe3c4fc8114b0a723, 0xbb0e6fd523adb9e8,
    0x8f295bebbd341415, 0xb43c7604f7a26cb7, 0xa4598bb726fa7417, 0x98df018e1984cbb2,
    0xc4ae345c802a8b12, 0x18533bf7dd051a8f, 0xd855f543aa101890, 0x73f458fd3ca20a7b,
    0x8955de42197dc029, 0xebd567cd3f114d43, 0xcf0b7c4dbb7da040, 0x97d0fbd6deaf63d2,
    0x522feba0337365a7, 0xc81a5609be7d6f06, 0xb1c5a91ffc13e604, 0x97095fddaca8455c,
    0x04ec2a2962e5b834, 0xc98084064297afdf, 0x0e8ea6f71750ffd2, 0x6d619b9823d2e22e,
    0xfc2b498af86a2138, 0xb5fcea5433b75939, 0x4abecdc610426d45, 0xb1b934e29de730d9,
    0x6285ed78715b3f07, 0x7e701e082d26a112, 0xfada81cca0c8ae8d, 0x0e434ff7fda9b763,
    0x9bcfd0913cfdba07, 0xb6cf83e36af959c8, 0x2016340ad3fa438b, 0x6d451003fb4b172e,
    0x10fdec124a68d757, 0xa3d3e7019a90604b, 0xdbc7f7c0c252d0b6, 0xe0aebdca8ed8dc54,
    0x7c7b5f50ed34705d, 0xb2f4d9eed340806f, 0xc40535da19aafb5d, 0xd68df8913e6ff156,
    0x07c425cbc83db596, 0xd66f5bf8fda90371, 0xa1c3cfc5f3333289, 0x539eb082389ab5cf,
    0x0cc42b06c6863aa4, 0x772f252a54a59b21, 0x22728d0c714e87a7, 0xd884b1720c9611d9,
    0xdc9ca8015eced8ab, 0xcfd2c804c9755dde, 0x5e1f742b263d8987, 0x1704189301ac7b5e,
    0x3c49116d049646e0, 0x4bfa71804d096baa, 0x97c52591e2a6aa59, 0x0326f5922f9661a9,
    0x354adcb609c444fa, 0x5b9f9e2edd3c7865, 0xf9f2af7d32f47f93, 0x0748e8c03e247fb7,
    0x7ef957e2824216f6, 0xd4dd3a1b4c5d667a, 0x49fb6450ff733fa9, 0x1e841816d55276d2,
    0xbe027dfb60b17121, 0x76b5f668e338dfe6, 0x9a227690f7de06c7, 0x7a6c5eb0c61146b6,
    0x84998179859fbb75, 0xf43bded162334af2, 0x3a48b9bccdab4931, 0x3e85cba55f4cc097,
    0xcafa73718db08c1c, 0xadc8afeef9e9ff3e, 0x259c0765de66d53f, 0x0d98c981ab0175f3,
    0x1ba1df279f35ac76, 0xcf7cb75c9bff472d, 0x3bea02c1da2e851a, 0xf1d9bb5e3b6652b1,
    0x9306afb78a4c2e17, 0x5125987cfb8850d2, 0x536170a205f994ff, 0x88205660246beafb,
    0x52d6c91af3219618, 0x4f69adcf11689705, 0xc8951d5767f006f9, 0x6bae37d19bcf2460,
    0x7fa3b054cabe3a0d, 0xd89c492e27163282, 0xf06e22169264e91d, 0x99dd47a11b4ce032,
    0x4b44937f43012e69, 0x05d1624dc1220e5f, 0x32313f6b4d585f89, 0xdda93772245fac4e,
    0x62af119cf65cd083, 0x67f796ebdfc0460d, 0x10e6851df123d3fe, 0x8c9145ad2a258e2b,
    0x0893eb45f31eccdf, 0xf5c238e2ccbfac98, 0xa5903d21be4f634e, 0xc827185779eac765,
    0x596c67b0fba18afd, 0x50f991751133ecbe, 0x01fd97a17e5ba8ff, 0x62c459d30742d2bf,
    0x5748d2f6ce89d41b, 0x68ff67608558a539, 0x52f10c6b519f2d47, 0xa17ef6d7dd6b528c,
    0x832e09f919f1d19e, 0x785190bba6310917, 0x988a42cdb93c67d6, 0xc31b2938ed73be43,
    0x85944e4cde9d42a3, 0xe1f6e967296998f4, 0x494740298137b045, 0xc28f78b839450b6a,
    0xf72333153b288fd3, 0x6de6379fdae18892, 0xf019c911e7f19924, 0xc6f06ab11454e0c9,
    0xbf131aabfd47fedd, 0xe02a3dcbd6966fa6, 0x074947d8a89de9f5, 0x460ca87c88302739,
    0x8adced696f62a1ae, 0x49cfacb843fe08fd, 0x216f5ec66091e6d2, 0xe08ee158ddee1c06,
    0x8df91a42add00477, 0xfdf50e76f1f8bf51, 0x1d4fa3cac632b586, 0x15f5725b8cbd2d27,
    0x9612cb0a58ea9007, 0x7b51a5dcfa4569a8, 0x27ce193baa2b8362, 0x6815d48a2a4c63f3,
    0xb6544cb28955b87b, 0x754f0187175c21d9, 0x427d6207844cf5e2, 0xc5f5e96f95b3cc5b,
    0x885d4e3ee8c4af20, 0x12cbcd1d6d93557c, 0x026ce1ce1e949953, 0xb930f3504fce07bd,
    0x761808f9e8cba37b, 0xaa60bdd29733e9a6, 0x2c246c5a77a17928, 0xe1c1c5b2157b2a03,
    0x4c74d71904a5fe47, 0xabb569fccc0141de, 0x16c32c4aafcb351a, 0x5b5619f4e8b28efe,
    0xc3834db2b3d4d6c9, 0x4ae5b8232af651b2, 0xb0c5456259b66be3, 0x7dc6d2211af523cb,
    0x2cc5671e366ceebe, 0xe438c0e3d15740b6, 0x1a50932d6e65e887, 0x8be246a968a14858,
    0x8b5af58fd341fde3, 0xb2c84f31d2aa0344, 0x93948f91456f0dcc, 0xad5a6cc0a1eda6cf,
    0xbf2251d9c991dbb5, 0x3489dfdc48210428, 0x22a1a7a8e1925f30, 0x069f879882a5a8e5,
    0x755e5f41e8316962, 0xb45f72e9de033d28, 0xb2949d43199fef6a, 0x4a8fa3eb16810d5b,
    0x4db6f7714f230f9f, 0x8206955348cd08e4, 0x2876b475f229ec78, 0xefe55a85a593595d,
    0x21d9173f0a753007, 0xfcc8cd013ae5097f, 0xcc3c2b3a61cfef84, 0x05afc8db7fe68212,
    0x9e29d99a6c8fc33b, 0x407c63999243a965, 0x8a2017b26b84fb95, 0xe2caf6992a6eb7c7,
    0x195a02f6e09e6144, 0x539a2c55affb863b, 0x4c80150cf9e32e85, 0xdb23c6e07cbd5b51,
    0x70d3ae4a5d84eb3b, 0xf798993a2ea01074, 0xf9558d6bf258ecbc, 0x6f57cccf61e437b6,
    0x33e1f9ead361cda8, 0xa86f7843ae7ac258, 0x9ecb9228d3638b83, 0xff18d674db5f9534,
    0x4bde2f948f9e3638, 0xa596ac3c03504b9d, 0x3a7da230fd7f2c6a, 0xc8aa7d11932f9cbe,
];

const fn fold_rr(v: u64) -> u64 {
    let v = (v >> 32) ^ v;
    (v >> 16) ^ v
}

const fn fold_rl(v: u64) -> u64 {
    let v = (v >> 32) ^ v;
    (v << 16) ^ v
}

const fn fold_lr(v: u64) -> u64 {
    let v = (v << 32) ^ v;
    (v >> 16) ^ v
}

const fn fold_ll(v: u64) -> u64 {
    let v = (v << 32) ^ v;
    (v << 16) ^ v
}

/// Gather the 8x8 bit-matrix product back into one word from its eight
/// folded partial rows.
const fn gather(r: [u64; 8]) -> u64 {
    let [p1, p2, p3, p4, p5, p6, p7, p8] = r;
    (p7 & 0x00ff_0000_0000_0000)
        ^ (((p1 & 0xff00) ^ (p3 & 0xff00_0000) ^ (p5 & 0xff00_0000_0000) ^ (p7 & 0xff00_0000_0000_00ff)) >> 8)
        ^ (((p2 & 0xff) ^ (p4 & 0x00ff_0000) ^ (p6 & 0xff_0000_0000)) << 8)
        ^ (((p8 << 8) ^ p8) & 0xff00_0000_0000_0000)
        ^ (p2 & 0xff00)
        ^ (p4 & 0xff00_0000)
        ^ (p6 & 0xff00_0000_0000)
        ^ (p1 & 0xff)
        ^ (p3 & 0x00ff_0000)
        ^ (p5 & 0xff_0000_0000)
}

/// Row `k` of the product: `h` masked by byte `k` of the matrix `m`.
const fn row(m: u64, h: u64, k: u32) -> u64 {
    STRETCH[((m >> (8 * k)) & 0xff) as usize] & h
}

/// Multiply the running hash `h` by one character's matrix `m`.
const fn mat_mul(m: u64, h: u64) -> u64 {
    gather([
        fold_rr(row(m, h, 0)),
        fold_rr(row(m, h, 1)),
        fold_rl(row(m, h, 2)),
        fold_rl(row(m, h, 3)),
        fold_lr(row(m, h, 4)),
        fold_lr(row(m, h, 5)),
        fold_ll(row(m, h, 6)),
        fold_ll(row(m, h, 7)),
    ])
}

/// The game's message-type hash of a C++ type name, read last character first.
pub const fn mat_sym(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut h = 0x8040_2010_0804_0201; // the identity matrix
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        h = mat_mul(MAT_SEEDS[bytes[i] as usize], h);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hashes() {
        // Values the game and its backends use.
        assert_eq!(mat_sym(""), 0x8040_2010_0804_0201);
        assert_eq!(symbol(""), u64::MAX);
    }
}
