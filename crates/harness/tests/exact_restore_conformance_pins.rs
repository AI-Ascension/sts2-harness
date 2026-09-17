// SPDX-License-Identifier: MIT

use serde_json::Value;

#[test]
fn exact_restore_conformance_pins_are_immutable_and_complete()
-> Result<(), Box<dyn std::error::Error>> {
    let bytes = include_bytes!("../../../tools/exact-restore-conformance/pins.json");
    let value: Value = serde_json::from_slice(bytes)?;
    assert_eq!(value["schema"], "sts2.exact-restore-conformance-pins.v1");
    for (field, repository) in [
        ("gateway_revision", "AI-Ascension/sts2-gateway"),
        ("mcp_revision", "AI-Ascension/sts2-mcp-server"),
        ("mod_revision", "AI-Ascension/sts2-game-mod"),
    ] {
        let repository_field = field.replace("_revision", "_repository");
        assert_eq!(value[repository_field.as_str()], repository);
        let revision = value[field]
            .as_str()
            .ok_or_else(|| format!("pins omitted {field}"))?;
        assert_eq!(revision.len(), 40);
        assert!(
            revision
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }
    Ok(())
}
