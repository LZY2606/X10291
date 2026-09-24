//! 单位/坐标基准：导入层保留原始表示，这里只在派生层做规范化。
//! 无法转换的值留在原位置并附诊断，绝不为了画图丢掉。

/// (scale, offset)：si = raw * scale + offset
fn conversion(unit: &str) -> Option<(f64, &'static str)> {
    Some(match unit {
        // 长度 → m
        "m" | "metre" | "meter" => (1.0, "m"),
        "mm" => (0.001, "m"),
        "cm" => (0.01, "m"),
        "km" => (1000.0, "m"),
        "in" | "inch" => (0.0254, "m"),
        // 温度 → K
        "K" | "kelvin" => (1.0, "K"),
        "degC" | "°C" => (1.0, "K"), // 需加偏移，下面特判
        // 压力 → Pa
        "Pa" => (1.0, "Pa"),
        "kPa" => (1000.0, "Pa"),
        "bar" => (100000.0, "Pa"),
        // 转速 → rad/s
        "rad/s" => (1.0, "rad/s"),
        "rpm" => (std::f64::consts::PI / 30.0, "rad/s"),
        // 角度 → rad
        "rad" => (1.0, "rad"),
        "deg" | "°" => (std::f64::consts::PI / 180.0, "rad"),
        _ => return None,
    })
}

/// 把原始值规范化到 SI 基准。无法转换时返回诊断，原值保留。
pub fn normalize_value(raw: &str, unit_raw: Option<&str>) -> (Option<f64>, Option<String>, Option<String>) {
    let value: f64 = match raw.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            return (
                None,
                None,
                Some(format!("value_not_numeric: 原始值 {:?} 不是数值，保留在原位置", raw)),
            )
        }
    };
    let unit = match unit_raw {
        Some(u) => u,
        None => return (Some(value), None, None), // 无单位：原样通过，基准未知
    };
    match unit {
        "degC" | "°C" => (Some(value + 273.15), Some("K".to_string()), None),
        "degF" | "°F" => (
            Some((value - 32.0) * 5.0 / 9.0 + 273.15),
            Some("K".to_string()),
            None,
        ),
        _ => match conversion(unit) {
            Some((scale, si)) => (Some(value * scale), Some(si.to_string()), None),
            None => (
                None,
                None,
                Some(format!(
                    "unit_unconvertible: 单位 {:?} 无换算规则，值 {:?} 保留在原位置",
                    unit, raw
                )),
            ),
        },
    }
}
