//! Stable, human-readable identifiers.
//!
//! These IDs never depend on insertion order or SQLite rowids, so exporting a
//! case, deleting the local database and re-importing it preserves identity,
//! ordering and source relations.

/// Percent-encode a string so it is safe inside a colon-separated id.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'~');
        if ok {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 64-bit FNV-1a, rendered as 16 hex chars — a short collision guard appended
/// to structured ids so that e.g. two nodes with identical encodings in
/// different files cannot silently collide.
pub fn fnv1a64(s: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn source_id(file: &str) -> String {
    format!("src:{}:{}", enc(file), &fnv1a64(file)[..8])
}

pub fn rawnode_id(file: &str, node_id: &str) -> String {
    let base = format!("{file}|{node_id}");
    format!("raw:{}:{}", enc(&base), &fnv1a64(&base)[..12])
}

/// Global node identity after URI-based namespace remap.
pub fn node_id(uri: &str, local: &str) -> String {
    format!("n:{}:{}", enc(uri), enc(local))
}

/// Forward normalized edge identity.
pub fn edge_id(src: &str, rel: &str, dst: &str) -> String {
    let base = format!("{src}->{rel}->{dst}");
    format!("e:{}:{}", enc(&base), &fnv1a64(&base)[..12])
}

pub fn issue_id(kind: &str, scope: &str) -> String {
    let base = format!("{kind}|{scope}");
    format!("iss:{}:{}", enc(&base), &fnv1a64(&base)[..12])
}
