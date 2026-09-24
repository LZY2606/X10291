pub fn convert(value: f64, unit: &str) -> Option<(f64, &'static str)> {
    match unit {
        "degC" | "°C" => Some((value + 273.15, "K")),
        "degF" | "°F" => Some(((value - 32.0) * 5.0 / 9.0 + 273.15, "K")),
        "K" => Some((value, "K")),
        "bar" => Some((value * 100_000.0, "Pa")),
        "Pa" => Some((value, "Pa")),
        "m" => Some((value, "m")),
        _ => None,
    }
}

pub fn convert_to(value: f64, from: &str, to: &str) -> Option<f64> {
    let (base, base_unit) = convert(value, from)?;
    if base_unit == to {
        return Some(base);
    }
    match (base_unit, to) {
        ("K", "degC") => Some(base - 273.15),
        ("K", "degF") => Some((base - 273.15) * 9.0 / 5.0 + 32.0),
        ("Pa", "bar") => Some(base / 100_000.0),
        _ => None,
    }
}
