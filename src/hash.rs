//! FNV-1 hashes as the games use them: over the ASCII-lowercased bytes.

const FNV32_OFFSET: u32 = 0x811c_9dc5;
const FNV32_PRIME: u32 = 0x0100_0193;
const FNV64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV64_PRIME: u64 = 0x0000_0100_0000_01b3;

pub fn fnv32(s: &str) -> u32 {
    s.bytes().fold(FNV32_OFFSET, |h, b| h.wrapping_mul(FNV32_PRIME) ^ u32::from(b.to_ascii_lowercase()))
}

pub fn fnv64(s: &str) -> u64 {
    s.bytes().fold(FNV64_OFFSET, |h, b| h.wrapping_mul(FNV64_PRIME) ^ u64::from(b.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(fnv32("lambert"), 0x9477_3578);
        assert_eq!(fnv32("diffuseMap"), 0x6cc0_fd85);
        assert_eq!(fnv64("fallback"), 0xc51f_614d_ef19_138f);
        // The mesh name both formats carry.
        assert_eq!(fnv32("none"), 0x2ca3_3bdb);
        assert_eq!(fnv32(""), FNV32_OFFSET);
    }
}
