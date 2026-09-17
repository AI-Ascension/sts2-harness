// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::fs;
use std::path::Path;

pub(crate) fn validate(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode pins: {error}"))?;
    for field in ["gateway_revision", "mcp_revision", "mod_revision"] {
        let revision = value[field]
            .as_str()
            .ok_or_else(|| format!("pins omitted {field}"))?;
        if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{field} is not a 40-hex revision"));
        }
    }
    Ok(())
}
