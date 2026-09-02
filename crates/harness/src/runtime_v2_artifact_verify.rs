// SPDX-License-Identifier: MIT

/// Verifies the copied release-like bytes before the fake runner uses them.
pub fn verify_runtime_v2_artifact() -> Result<(), RuntimeV2ArtifactError> {
    let manifest: Value = serde_json::from_slice(MANIFEST_BYTES)
        .map_err(|_| RuntimeV2ArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(RuntimeV2ArtifactError::ManifestMismatch);
    }

    let schema: Value =
        serde_json::from_slice(SCHEMA_BYTES).map_err(|_| RuntimeV2ArtifactError::SchemaMismatch)?;
    if !schema_matches(&schema) || SCHEMA_BYTES != SOURCE_SCHEMA_BYTES {
        return Err(RuntimeV2ArtifactError::SchemaMismatch);
    }

    if !checksums_match() {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
    }

    let schema_digest = Sha256::digest(SCHEMA_BYTES);
    if format!("{schema_digest:x}") != RUNTIME_V2_SCHEMA_DIGEST {
        return Err(RuntimeV2ArtifactError::ChecksumMismatch);
    }

    Ok(())
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    if object.len() != 8
        || ![
            "artifact",
            "protocol_version",
            "schema",
            "schema_digest",
            "provenance",
            "consumers",
            "goldens",
            "checksums",
        ]
        .into_iter()
        .all(|key| object.contains_key(key))
    {
        return false;
    }

    if string_field(manifest, "artifact") != Some(RUNTIME_V2_ARTIFACT)
        || string_field(manifest, "protocol_version") != Some(RUNTIME_V2_PROTOCOL_VERSION)
        || string_field(manifest, "schema") != Some("schema.json")
        || string_field(manifest, "schema_digest") != Some(RUNTIME_V2_SCHEMA_DIGEST)
    {
        return false;
    }

    let Some(provenance) = manifest.get("provenance").and_then(Value::as_object) else {
        return false;
    };
    if provenance.len() != 3
        || provenance.get("source").and_then(Value::as_str) != Some(RUNTIME_V2_SCHEMA_SOURCE)
        || provenance.get("generator").and_then(Value::as_str) != Some(RUNTIME_V2_GENERATOR)
        || provenance.get("license").and_then(Value::as_str) != Some("MIT")
    {
        return false;
    }

    let consumers_match = manifest
        .get("consumers")
        .and_then(Value::as_array)
        .is_some_and(|consumers| {
            consumers.len() == 4
                && consumers
                    .iter()
                    .zip([
                        "sts2-game-mod",
                        "sts2-gateway",
                        "sts2-harness",
                        "sts2-mcp-server",
                    ])
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        });
    let goldens_match = manifest
        .get("goldens")
        .and_then(Value::as_array)
        .is_some_and(|goldens| {
            goldens.len() == GOLDEN_PATHS.len()
                && goldens
                    .iter()
                    .zip(MANIFEST_GOLDEN_PATHS)
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        });
    consumers_match && goldens_match && string_field(manifest, "checksums") == Some("SHA256SUMS")
}

fn schema_matches(schema: &Value) -> bool {
    let Some(definitions) = schema.get("$defs").and_then(Value::as_object) else {
        return false;
    };
    let Some(base) = definitions.get("base").and_then(Value::as_object) else {
        return false;
    };
    let Some(base_required) = base.get("required").and_then(Value::as_array) else {
        return false;
    };
    let Some(action) = definitions.get("action").and_then(Value::as_object) else {
        return false;
    };
    let Some(action_properties) = action.get("properties").and_then(Value::as_object) else {
        return false;
    };
    let Some(observation) = definitions.get("observation").and_then(Value::as_object) else {
        return false;
    };
    let Some(observation_properties) = observation.get("properties").and_then(Value::as_object)
    else {
        return false;
    };
    let Some(kinds) = schema.get("oneOf").and_then(Value::as_array) else {
        return false;
    };

    schema.get("$id").and_then(Value::as_str) == Some("sts2-runtime-v2")
        && kinds.len() == 6
        && [
            "protocol_version",
            "schema_digest",
            "provenance",
            "correlation_id",
            "instance_id",
            "session_id",
            "lease_id",
            "lease_epoch",
            "generation",
        ]
        .into_iter()
        .all(|field| {
            base_required
                .iter()
                .any(|value| value.as_str() == Some(field))
        })
        && action_properties
            .get("action_id")
            .and_then(|value| value.get("const"))
            .and_then(Value::as_str)
            == Some("end_turn")
        && observation_properties
            .get("turn_index")
            .and_then(|value| value.get("maximum"))
            .and_then(Value::as_u64)
            == Some(RUNTIME_V2_MAX_TURN_INDEX)
        && observation_properties
            .get("generation")
            .and_then(|value| value.get("maximum"))
            .and_then(Value::as_u64)
            == Some(RUNTIME_V2_MAX_GENERATION)
}

fn checksums_match() -> bool {
    let expected = [
        ("../../conformance/cases/runtime-v2.json", CONFORMANCE_BYTES),
        ("../../schemas/runtime-v2.schema.json", SOURCE_SCHEMA_BYTES),
        ("manifest.json", MANIFEST_BYTES),
        ("schema.json", SCHEMA_BYTES),
    ]
    .into_iter()
    .chain(GOLDEN_PATHS.into_iter().zip(GOLDEN_BYTES))
    .collect::<Vec<_>>();
    let mut seen = vec![false; expected.len()];
    let mut listed = 0;
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        let Some(index) = expected
            .iter()
            .position(|(expected_path, _)| *expected_path == path)
        else {
            return false;
        };
        if seen[index]
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || format!("{:x}", Sha256::digest(expected[index].1)) != digest
        {
            return false;
        }
        seen[index] = true;
        listed += 1;
    }
    listed == expected.len() && seen.into_iter().all(|present| present)
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}
