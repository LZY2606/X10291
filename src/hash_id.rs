//! Stable content-addressed identifiers.
//!
//! Identities are derived from content (SHA-256 hex prefix) so that an export
//! deleted from disk and re-imported yields the *same* ids, ordering and
//! provenance relations. They are never assigned from an autoincrement column.

use sha2::{Digest, Sha256};

/// Compute a stable 16-hex-char id from the joined parts (`part\n` each).
pub fn hid(prefix: &str, parts: &[&str]) -> String {
    let mut h = Sha256::new();
    h.update(prefix.as_bytes());
    h.update(b"\x00");
    for p in parts {
        h.update(p.as_bytes());
        h.update(b"\n");
    }
    let digest = h.finalize();
    format!("{}_{}", prefix, hex::encode(&digest[..8]))
}

/// Convenience wrapper over string references.
pub fn hid_owned(prefix: &str, parts: &[String]) -> String {
    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
    hid(prefix, &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_order_sensitive() {
        assert_eq!(hid("x", &["a", "b"]), hid("x", &["a", "b"]));
        assert_ne!(hid("x", &["a", "b"]), hid("x", &["b", "a"]));
        assert_ne!(hid("x", &["a"]), hid("y", &["a"]));
        assert!(hid("src", &["z"]).starts_with("src_"));
    }
}
