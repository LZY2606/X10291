//! Domain quantities: every value carries a unit and a coordinate basis.
//! The raw representation is authoritative and never rewritten; normalization
//! happens only in the derived layer. Values that cannot be converted stay in
//! place and carry a diagnostic.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quantity {
    pub raw_value: String,
    pub unit: String,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedQuantity {
    pub raw: Quantity,
    pub normalized_value: Option<f64>,
    pub normalized_unit: Option<String>,
    /// None when conversion succeeded.
    pub diagnostic: Option<String>,
}

/// Parse a `<Quantity value=".." unit=".." basis=".."/>` extension body.
pub fn parse_quantity_extension(raw_xml: &str) -> Option<Quantity> {
    if !raw_xml.contains("Quantity") {
        return None;
    }
    let attr = |name: &str| -> Option<String> {
        let key = format!("{name}=\"");
        let start = raw_xml.find(&key)? + key.len();
        let end = raw_xml[start..].find('"')? + start;
        Some(raw_xml[start..end].to_string())
    };
    Some(Quantity {
        raw_value: attr("value")?,
        unit: attr("unit").unwrap_or_default(),
        basis: attr("basis").unwrap_or_else(|| "unspecified".to_string()),
    })
}

fn convert(value: f64, unit: &str) -> Option<(f64, &'static str)> {
    Some(match unit {
        "degC" => (value + 273.15, "K"),
        "degF" => ((value - 32.0) * 5.0 / 9.0 + 273.15, "K"),
        "K" => (value, "K"),
        "mm" => (value / 1000.0, "m"),
        "cm" => (value / 100.0, "m"),
        "in" => (value * 0.0254, "m"),
        "m" => (value, "m"),
        "rpm" => (value / 60.0, "Hz"),
        "Hz" => (value, "Hz"),
        "percent" => (value, "percent"),
        _ => return None,
    })
}

pub fn normalize(q: &Quantity) -> NormalizedQuantity {
    let parsed: Result<f64, _> = q.raw_value.trim().parse();
    match parsed {
        Err(_) => NormalizedQuantity {
            raw: q.clone(),
            normalized_value: None,
            normalized_unit: None,
            diagnostic: Some(format!(
                "value '{}' is not numeric; raw kept in place",
                q.raw_value
            )),
        },
        Ok(v) => match convert(v, &q.unit) {
            Some((nv, nu)) => NormalizedQuantity {
                raw: q.clone(),
                normalized_value: Some(nv),
                normalized_unit: Some(nu.to_string()),
                diagnostic: None,
            },
            None => NormalizedQuantity {
                raw: q.clone(),
                normalized_value: None,
                normalized_unit: None,
                diagnostic: Some(format!(
                    "unit '{}' has no conversion to SI; raw kept in place",
                    q.unit
                )),
            },
        },
    }
}
