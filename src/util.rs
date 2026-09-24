pub fn fnv64(data: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hex64(v: u64) -> String {
    format!("{:016x}", v)
}

pub fn id_hash(parts: &[&str]) -> String {
    hex64(fnv64(&parts.join("\u{1f}")))
}

pub fn file_id(name: &str, xml: &str) -> String {
    format!("file:{}", id_hash(&[name, xml]))
}

pub fn sha(xml: &str) -> String {
    hex64(fnv64(xml))
}

pub fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
