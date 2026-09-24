//! Deterministic, export-stable content identities (FNV-1a 64 bit).
//!
//! Identities are derived solely from stable content (URI-keyed NodeIds,
//! source spans, raw text) so they survive export, database deletion and
//! re-import without depending on SQLite rowids.

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut material = String::new();
    for p in parts {
        material.push_str(p);
        material.push('\u{1f}');
    }
    format!("{prefix}_{:016x}", fnv1a64(material.as_bytes()))
}
