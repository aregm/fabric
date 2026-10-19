//! The git-ignored `.env` file (Q25): `KEY=VALUE` lines. Values are never printed.

use std::collections::HashMap;
use std::path::Path;

pub fn read(path: &Path) -> std::io::Result<HashMap<String, String>> {
    let text = std::fs::read_to_string(path)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut values = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(
                key.trim().to_owned(),
                value.trim().trim_matches(['"', '\'']).to_owned(),
            );
        }
    }
    Ok(values)
}
