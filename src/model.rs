//! Core OPC UA address-space value types and canonicalization rules.
//!
//! Semantics enforced here:
//! * A numeric namespace index is only meaningful inside a single file; the
//!   canonical form of a NodeId/BrowseName always carries the namespace URI.
//! * Raw textual representations are never rewritten at import time; canonical
//!   forms live in the derived layer only.

pub const CORE_NS_URI: &str = "http://opcfoundation.org/UA/";

/// Parsed NodeId text like `ns=2;i=47` or `i=58`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeIdText {
    pub ns: u16,
    /// Canonical id part, e.g. `i=47`, `s=Motor1`, `g=...`, `b=...`.
    pub id_part: String,
}

/// Parse the textual NodeId form. Returns None when the text is not a valid
/// NodeId literal (callers keep the raw text and raise a diagnostic).
pub fn parse_node_id(text: &str) -> Option<NodeIdText> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut ns: u16 = 0;
    let mut rest = text;
    if let Some((head, tail)) = text.split_once(';') {
        let head = head.trim();
        if let Some(num) = head.strip_prefix("ns=") {
            ns = num.parse().ok()?;
            rest = tail.trim();
        } else if head.starts_with("nsu=") {
            // URI-form NodeIds are not produced by our fixtures; keep raw.
            return None;
        }
    }
    for prefix in ["i=", "s=", "g=", "b="] {
        if let Some(v) = rest.strip_prefix(prefix) {
            if v.is_empty() {
                return None;
            }
            if prefix == "i=" && v.parse::<u64>().is_err() {
                return None;
            }
            return Some(NodeIdText {
                ns,
                id_part: format!("{}{}", prefix, v),
            });
        }
    }
    None
}

/// Canonical, merge-wide identity of a NodeId: namespace URI + id part.
pub fn canonical_node_id(ns_uri: &str, id_part: &str) -> String {
    format!("nsu={};{}", ns_uri, id_part)
}

/// Parse a BrowseName like `2:MotorType` -> (2, "MotorType").
pub fn parse_browse_name(text: &str) -> (u16, String) {
    let text = text.trim();
    if let Some((head, tail)) = text.split_once(':') {
        if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(ns) = head.parse::<u16>() {
                return (ns, tail.to_string());
            }
        }
    }
    (0, text.to_string())
}

pub fn canonical_browse_name(ns_uri: &str, name: &str) -> String {
    format!("nsu={};name={}", ns_uri, name)
}

/// Resolve alias chains (file-scoped). Returns Err(chain) on a cycle; the
/// caller keeps the raw text and records an `alias_cycle` event.
pub fn resolve_alias(
    text: &str,
    aliases: &std::collections::HashMap<String, String>,
) -> Result<String, Vec<String>> {
    let mut cur = text.trim().to_string();
    let mut chain: Vec<String> = Vec::new();
    for _ in 0..64 {
        match aliases.get(&cur) {
            Some(next) => {
                if chain.iter().any(|c| c == &cur) {
                    chain.push(cur);
                    return Err(chain);
                }
                chain.push(cur.clone());
                cur = next.trim().to_string();
            }
            None => return Ok(cur),
        }
    }
    chain.push(cur);
    Err(chain)
}

/// A physical quantity keeping its raw representation plus an optional
/// derived SI normalization. Unconvertible values stay raw with a diagnostic.
#[derive(Debug, Clone)]
pub struct Quantity {
    pub raw_value: String,
    pub raw_unit: String,
    pub frame: Option<String>,
    pub norm_value: Option<f64>,
    pub norm_unit: Option<String>,
    pub diagnostic: Option<String>,
}

/// Tiny, explicit unit table. Anything outside the table is NOT converted;
/// the raw value stays in place and a diagnostic is attached.
pub fn normalize_quantity(raw_value: &str, raw_unit: &str, frame: Option<&str>) -> Quantity {
    let table: &[(&str, &str, f64)] = &[
        ("N·m", "N·m", 1.0),
        ("N*m", "N·m", 1.0),
        ("lbf·ft", "N·m", 1.3558179483314004),
        ("lbf*ft", "N·m", 1.3558179483314004),
        ("mm", "m", 0.001),
        ("in", "m", 0.0254),
        ("m", "m", 1.0),
        ("kg", "kg", 1.0),
        ("g", "kg", 0.001),
    ];
    let parsed = raw_value.trim().parse::<f64>();
    let found = table.iter().find(|(u, _, _)| *u == raw_unit.trim());
    let (norm_value, norm_unit, diagnostic) = match (parsed, found) {
        (Ok(v), Some((_, si, factor))) => (Some(v * factor), Some(si.to_string()), None),
        (Ok(_), None) => (
            None,
            None,
            Some(format!(
                "unit '{}' not in conversion table; raw value kept",
                raw_unit
            )),
        ),
        (Err(_), _) => (
            None,
            None,
            Some(format!("value '{}' is not numeric; raw value kept", raw_value)),
        ),
    };
    Quantity {
        raw_value: raw_value.to_string(),
        raw_unit: raw_unit.to_string(),
        frame: frame.map(|f| f.to_string()),
        norm_value,
        norm_unit,
        diagnostic,
    }
}

/// FNV-1a hex digest — stable identity helper, no external deps.
pub fn fnv_hex(data: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in data.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_node_ids() {
        assert_eq!(
            parse_node_id("ns=2;i=47").unwrap(),
            NodeIdText { ns: 2, id_part: "i=47".into() }
        );
        assert_eq!(parse_node_id("i=58").unwrap().ns, 0);
        assert_eq!(
            parse_node_id("ns=1;s=Motor 1").unwrap().id_part,
            "s=Motor 1"
        );
        assert!(parse_node_id("ns=x;i=1").is_none());
        assert!(parse_node_id("hello").is_none());
    }

    #[test]
    fn browse_names() {
        assert_eq!(parse_browse_name("2:MotorType"), (2, "MotorType".to_string()));
        assert_eq!(parse_browse_name("Motor"), (0, "Motor".to_string()));
        assert_eq!(parse_browse_name("1:ns:weird"), (1, "ns:weird".to_string()));
    }

    #[test]
    fn alias_cycle_detected() {
        let mut m = std::collections::HashMap::new();
        m.insert("A".to_string(), "B".to_string());
        m.insert("B".to_string(), "A".to_string());
        assert!(resolve_alias("A", &m).is_err());
        m.insert("C".to_string(), "i=47".to_string());
        assert_eq!(resolve_alias("C", &m).unwrap(), "i=47");
    }

    #[test]
    fn units() {
        let q = normalize_quantity("10", "lbf·ft", None);
        assert!((q.norm_value.unwrap() - 13.558179483314004).abs() < 1e-9);
        let q = normalize_quantity("42.5", "rpm", None);
        assert!(q.norm_value.is_none());
        assert!(q.diagnostic.unwrap().contains("not in conversion table"));
    }
}
