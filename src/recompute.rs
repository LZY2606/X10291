use rusqlite::Connection;

/// 派生计算缓存：按输入指纹判断“仍有效”或“需要重算”，并给出可说明的理由。
/// 重算沿依赖传播——只有输入指纹变化的派生项才会重算。
pub fn ensure_derived(
    conn: &Connection,
    key: &str,
    inputs: &[(String, String)],
    compute: impl FnOnce() -> String,
) -> (String, String) {
    let inputs_json = serde_json::to_string(inputs).unwrap_or_default();
    let row: Option<(String, String)> = conn
        .query_row("SELECT inputs, value FROM derived WHERE key=?1", [key], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .ok();
    let summarize = |inputs: &[(String, String)]| {
        inputs
            .iter()
            .map(|(k, v)| format!("{k}={}", &v.chars().take(12).collect::<String>()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match row {
        Some((old_inputs, value)) if old_inputs == inputs_json => {
            (value, format!("仍然有效：输入未变化（{}），跳过重算", summarize(inputs)))
        }
        Some((old_inputs, _)) => {
            let value = compute();
            let _ = conn.execute(
                "INSERT OR REPLACE INTO derived(key, inputs, value) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, inputs_json, value],
            );
            let old: Vec<(String, String)> = serde_json::from_str(&old_inputs).unwrap_or_default();
            let changed: Vec<String> = inputs
                .iter()
                .filter(|(k, v)| old.iter().find(|(ok, _)| ok == k).map(|(_, ov)| ov != v).unwrap_or(true))
                .map(|(k, _)| k.clone())
                .collect();
            (value, format!("已重算：受影响的输入 [{}] 发生变化", changed.join(", ")))
        }
        None => {
            let value = compute();
            let _ = conn.execute(
                "INSERT OR REPLACE INTO derived(key, inputs, value) VALUES (?1, ?2, ?3)",
                rusqlite::params![key, inputs_json, value],
            );
            (value, "首次计算：无历史缓存".to_string())
        }
    }
}
