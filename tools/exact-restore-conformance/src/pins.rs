// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::fs;
use std::path::Path;

pub(crate) fn validate(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode pins: {error}"))?;
    for (field, repository) in [
        ("gateway_revision", "AI-Ascension/sts2-gateway"),
        ("mcp_revision", "AI-Ascension/sts2-mcp-server"),
        ("mod_revision", "AI-Ascension/sts2-game-mod"),
    ] {
        let repository_field =
            field.strip_suffix("_revision").unwrap_or(field).to_owned() + "_repository";
        if value[repository_field.as_str()] != repository {
            return Err(format!(
                "{repository_field} does not match the pinned source"
            ));
        }
        let revision = value[field]
            .as_str()
            .ok_or_else(|| format!("pins omitted {field}"))?;
        if revision.len() != 40
            || !revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!("{field} is not a 40-hex revision"));
        }
    }
    Ok(())
}
