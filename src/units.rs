use crate::merge::MergedModel;
use serde::{Deserialize, Serialize};

/// 派生层的量值规范化结果。原始表示始终留在节点属性里，
/// 无法转换的值留在原位置并附诊断，不为了画图丢掉。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NormalizedQuantity {
    pub node: String,
    pub raw_value: String,
    pub raw_unit: String,
    pub canonical_value: Option<f64>,
    pub canonical_unit: Option<String>,
    pub diagnostic: Option<String>,
}

/// (倍率, 偏移, 规范单位)：canonical = raw * factor + offset。
fn conversion(unit: &str) -> Option<(f64, f64, &'static str)> {
    match unit {
        "m" => Some((1.0, 0.0, "m")),
        "mm" => Some((0.001, 0.0, "m")),
        "km" => Some((1000.0, 0.0, "m")),
        "m/s" => Some((1.0, 0.0, "m/s")),
        "km/h" => Some((1.0 / 3.6, 0.0, "m/s")),
        "Pa" => Some((1.0, 0.0, "Pa")),
        "kPa" => Some((1000.0, 0.0, "Pa")),
        "MPa" => Some((1.0e6, 0.0, "Pa")),
        "K" => Some((1.0, 0.0, "K")),
        "°C" => Some((1.0, 273.15, "K")),
        "A" => Some((1.0, 0.0, "A")),
        "mA" => Some((0.001, 0.0, "A")),
        _ => None,
    }
}

pub fn normalize_quantities(model: &MergedModel) -> Vec<NormalizedQuantity> {
    let mut out = Vec::new();
    for (id, node) in &model.nodes {
        let get = |k: &str| node.attributes.iter().find(|(a, _)| a == k).map(|(_, v)| v.clone());
        let (Some(value), Some(unit)) = (get("Value"), get("Unit")) else { continue };
        let parsed = value.parse::<f64>();
        let conv = conversion(&unit);
        let (canonical_value, canonical_unit, diagnostic) = match (parsed, conv) {
            (Ok(v), Some((factor, offset, canon))) => (Some(v * factor + offset), Some(canon.to_string()), None),
            (Err(_), _) => (
                None,
                None,
                Some(format!("值 `{value}` 不是数值，保留原始表示 `{value} {unit}`")),
            ),
            (Ok(_), None) => (
                None,
                None,
                Some(format!("单位 `{unit}` 无转换规则，保留原始表示 `{value} {unit}`")),
            ),
        };
        out.push(NormalizedQuantity {
            node: id.clone(),
            raw_value: value,
            raw_unit: unit,
            canonical_value,
            canonical_unit,
            diagnostic,
        });
    }
    out
}
