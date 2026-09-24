use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct CaseSourceFile {
    pub version_id: String,
    pub path: String,
    pub label: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct CaseManifest {
    pub versions: Vec<CaseVersionSpec>,
}

#[derive(Debug, Clone)]
pub struct CaseVersionSpec {
    pub id: String,
    pub branch_id: String,
    pub label: String,
    pub parent_id: Option<String>,
    pub changed_sources: Vec<String>,
    pub source_paths: Vec<String>,
}

pub fn stable_id(parts: &[&str]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xfe;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

pub fn read_file(root: &Path, relative: &str) -> Result<String, String> {
    let path = root.join(relative);
    let mut file = File::open(&path).map_err(|err| format!("open {}: {}", path.display(), err))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|err| format!("read {}: {}", path.display(), err))?;
    Ok(content)
}

pub fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("create dir: {err}"))?;
    }
    let mut file = File::create(path).map_err(|err| format!("create {}: {err}", path.display()))?;
    file.write_all(content.as_bytes())
        .map_err(|err| format!("write {}: {err}", path.display()))?;
    Ok(())
}
