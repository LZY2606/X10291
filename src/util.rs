use serde::Serialize;
use std::collections::BTreeMap;

pub fn stable_id(parts: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0x1f;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:x}", hash)
}

pub fn sha_like(content: &str) -> String {
    let mut hash = 0x6a09e667f3bcc908u64;
    for (index, byte) in content.bytes().enumerate() {
        hash = hash
            .rotate_left((index % 57) as u32 + 5)
            .wrapping_add(byte as u64)
            .wrapping_mul(0x9e3779b97f4a7c15);
    }
    format!("sha256-like-{hash:016x}")
}

pub fn deterministic_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn sorted_map(pairs: Vec<(&str, &str)>) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}
