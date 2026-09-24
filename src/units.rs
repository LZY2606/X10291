//! Engineering-unit handling. Raw representations are kept on import; only the
//! derived layer normalizes. Unconvertible values stay in place with a
//! diagnostic — they are never dropped to make a plot prettier.

use crate::model::{EUInfo, NormValue};

/// (display name, factor to SI, offset to SI, SI symbol)
const CONVERSIONS: &[(&str, f64, f64, &str)] = &[
    ("Pa", 1.0, 0.0, "Pa"),
    ("kPa", 1_000.0, 0.0, "Pa"),
    ("MPa", 1_000_000.0, 0.0, "Pa"),
    ("bar", 100_000.0, 0.0, "Pa"),
    ("K", 1.0, 0.0, "K"),
    ("°C", 1.0, 273.15, "K"),
    ("m", 1.0, 0.0, "m"),
    ("mm", 0.001, 0.0, "m"),
    ("rad", 1.0, 0.0, "rad"),
    ("rad/s", 1.0, 0.0, "rad/s"),
    ("rpm", std::f64::consts::PI / 30.0, 0.0, "rad/s"),
];

/// Try to normalize `raw_text` with `unit` into SI. Returns None when the value
/// or unit cannot be converted; callers keep the raw value and add a diagnostic.
pub fn normalize(raw_text: &str, unit: &EUInfo) -> Result<NormValue, String> {
    let value: f64 = raw_text
        .trim()
        .parse()
        .map_err(|_| format!("value '{}' is not numeric; kept in original position", raw_text))?;
    let name = unit.display_name.trim();
    match CONVERSIONS.iter().find(|(n, _, _, _)| *n == name) {
        Some((_, factor, offset, si)) => Ok(NormValue { value: value * factor + offset, unit: si.to_string() }),
        None => Err(format!(
            "no conversion registered for unit '{}' (unitId={}, {}); value kept in original position",
            name, unit.unit_id, unit.description
        )),
    }
}
