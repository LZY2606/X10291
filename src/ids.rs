//! Stable content-based identities. Every displayed value carries one of these;
//! they survive export / delete / re-import because they depend only on content.

/// FNV-1a 64-bit, hex-encoded. Deterministic, no external deps.
pub fn fnv1a(data: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Stable identity for a domain object, e.g. stable_id("node", "urn:acme:a#i=5001").
pub fn stable_id(kind: &str, canonical: &str) -> String {
    format!("{}_{:016x}", kind, fnv1a(&format!("{}:{}", kind, canonical)))
}

/// Short content hash for files.
pub fn content_hash(content: &str) -> String {
    format!("{:016x}", fnv1a(content))
}
