//! Stable identity helpers. All derived ids are content-addressed (FNV-1a 64)
//! so export -> delete -> import preserves identity, order and provenance.

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in parts {
        h ^= fnv1a64(p.as_bytes());
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{prefix}_{h:016x}")
}
