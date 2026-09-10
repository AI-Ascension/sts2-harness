// SPDX-License-Identifier: MIT

/// Verify the exact copied component and return its admission state.
pub fn verify_coop_native_artifact_state(
) -> Result<CoopNativeArtifactState, CoopNativeArtifactError> {
    let manifest: Value =
        serde_json::from_slice(MANIFEST).map_err(|_| CoopNativeArtifactError::ManifestMismatch)?;
    if !manifest_matches(&manifest) {
        return Err(CoopNativeArtifactError::ManifestMismatch);
    }

    let schema: Value =
        serde_json::from_slice(SCHEMA).map_err(|_| CoopNativeArtifactError::SchemaMismatch)?;
    if SCHEMA != SOURCE_SCHEMA
        || sha256(SCHEMA) != COOP_NATIVE_SCHEMA_DIGEST
        || !schema_matches(&schema)
    {
        return Err(CoopNativeArtifactError::SchemaMismatch);
    }

    let conformance: Value = serde_json::from_slice(CONFORMANCE)
        .map_err(|_| CoopNativeArtifactError::ConformanceMismatch)?;
    let conformance_copy: Value = serde_json::from_slice(CONFORMANCE_COPY)
        .map_err(|_| CoopNativeArtifactError::ConformanceMismatch)?;
    if conformance != conformance_copy || !conformance_matches(&conformance) {
        return Err(CoopNativeArtifactError::ConformanceMismatch);
    }
    let consumer: Value = serde_json::from_slice(CONSUMER_CONFORMANCE)
        .map_err(|_| CoopNativeArtifactError::ConformanceMismatch)?;
    if !consumer_conformance_matches(&consumer) {
        return Err(CoopNativeArtifactError::ConformanceMismatch);
    }
    if serde_json::from_slice::<Value>(PRODUCER_CAPTURE).is_err() {
        return Err(CoopNativeArtifactError::GoldenMismatch);
    }
    if GOLDENS
        .iter()
        .any(|(_, bytes)| serde_json::from_slice::<Value>(bytes).is_err())
    {
        return Err(CoopNativeArtifactError::GoldenMismatch);
    }
    if !checksums_match() {
        return Err(CoopNativeArtifactError::ChecksumMismatch);
    }
    Ok(CoopNativeArtifactState {
        status: CoopNativeArtifactStatus::AcceptedComponent,
        admission: CoopNativeAdmissionStatus::Component,
        producer_digest_matches_candidate: true,
    })
}

/// Compatibility name retained for callers that previously consumed the candidate adapter.
pub fn verify_coop_native_candidate_artifact(
) -> Result<CoopNativeArtifactState, CoopNativeArtifactError> {
    verify_coop_native_artifact_state()
}

/// Verify the accepted component before parsing or coordinating an envelope.
pub fn verify_coop_native_artifact() -> Result<(), CoopNativeArtifactError> {
    let state = verify_coop_native_artifact_state()?;
    if !state.producer_digest_matches_candidate() {
        return Err(CoopNativeArtifactError::ProducerDigestMismatch);
    }
    if !state.is_admitted() {
        return Err(CoopNativeArtifactError::Unadmitted);
    }
    Ok(())
}

#[must_use]
pub fn coop_native_schema_bytes() -> &'static [u8] {
    SCHEMA
}

#[must_use]
pub fn coop_native_manifest_bytes() -> &'static [u8] {
    MANIFEST
}

fn manifest_matches(manifest: &Value) -> bool {
    let Some(object) = manifest.as_object() else {
        return false;
    };
    if object.len() != 20 {
        return false;
    }
    let exact = |key: &str, value: &str| object.get(key).and_then(Value::as_str) == Some(value);
    exact("artifact", COOP_NATIVE_ARTIFACT)
        && exact("protocol_version", COOP_NATIVE_PROTOCOL_VERSION)
        && exact("status", "accepted_component")
        && exact("admission", "component")
        && exact("live_status", "unverified")
        && exact("live_gate", "pending_native_two_peer_settlement")
        && exact("schema", "schema.json")
        && exact("schema_digest", COOP_NATIVE_SCHEMA_DIGEST)
        && exact("producer_declared_schema_digest", COOP_NATIVE_PRODUCER_SCHEMA_DIGEST)
        && object.get("producer_digest_matches_candidate") == Some(&Value::Bool(true))
        && exact("producer_capture", "producer-capture.json")
        && exact("consumer_conformance", "consumer-conformance.json")
        && exact("consumer_conformance_status", "component_serialized_conformance")
        && exact("conformance", "conformance.json")
        && exact("checksums", "SHA256SUMS")
        && provenance_matches(object.get("provenance"))
        && object.get("consumers") == Some(&Value::Array(
            ["sts2-gateway", "sts2-mcp-server", "sts2-harness"]
                .into_iter()
                .map(|value| Value::String(value.to_owned()))
                .collect(),
        ))
        && goldens_manifest_matches(object.get("goldens"))
}

fn provenance_matches(value: Option<&Value>) -> bool {
    let Some(object) = value.and_then(Value::as_object) else {
        return false;
    };
    object.len() == 3
        && object.get("source").and_then(Value::as_str) == Some(COOP_NATIVE_SCHEMA_SOURCE)
        && object.get("generator").and_then(Value::as_str) == Some(COOP_NATIVE_GENERATOR)
        && object.get("license").and_then(Value::as_str) == Some("MIT")
}

fn goldens_manifest_matches(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|goldens| {
        goldens.len() == GOLDENS.len()
            && goldens
                .iter()
                .zip(GOLDENS)
                .all(|(actual, (path, _))| actual.as_str() == Some(path))
    })
}

fn schema_matches(schema: &Value) -> bool {
    schema.get("$id").and_then(Value::as_str) == Some("sts2-coop-native-v1-candidate")
        && schema.get("additionalProperties") == Some(&Value::Bool(false))
        && schema.get("required").and_then(Value::as_array).is_some_and(|fields| {
            fields.len() == 20
                && fields.iter().all(Value::is_string)
                && fields.iter().filter_map(Value::as_str).all(|field| {
                    [
                        "protocol_version",
                        "schema_digest",
                        "provenance",
                        "correlation_id",
                        "instance_id",
                        "session_id",
                        "lease_id",
                        "lease_epoch",
                        "kind",
                        "operation_id",
                        "actor_peer",
                        "expected_host_generation",
                        "action",
                        "vote",
                        "status",
                        "observation",
                        "effect",
                        "recovery",
                        "catalog",
                        "receipt",
                    ]
                    .contains(&field)
                })
        })
}

fn conformance_matches(conformance: &Value) -> bool {
    conformance.get("contract").and_then(Value::as_str)
        == Some("sts2.protocol/coop-native-v1")
        && conformance.get("protocol_version").and_then(Value::as_str)
            == Some(COOP_NATIVE_PROTOCOL_VERSION)
        && conformance.get("status").and_then(Value::as_str) == Some("accepted_component")
        && conformance.get("schema_digest").and_then(Value::as_str)
            == Some(COOP_NATIVE_SCHEMA_DIGEST)
        && conformance
            .get("producer_declared_schema_digest")
            .and_then(Value::as_str)
            == Some(COOP_NATIVE_PRODUCER_SCHEMA_DIGEST)
        && conformance.get("cases").and_then(Value::as_array).is_some_and(|cases| {
            cases.len() == GOLDENS.len()
                && cases.iter().zip(GOLDENS).all(|(case, (path, _))| {
                    case.get("name").and_then(Value::as_str)
                        == Some(path.strip_suffix(".json").unwrap_or(path))
                        && case.get("fixture").and_then(Value::as_str) == Some(path)
                        && case.get("schema_valid") == Some(&Value::Bool(true))
                })
        })
        && conformance
            .get("negative_mutations")
            .and_then(Value::as_array)
            .is_some_and(|values| values.len() >= 10 && values.iter().all(Value::is_string))
        && conformance
            .get("serialized_conformance")
            .and_then(Value::as_object)
            .is_some_and(|value| {
                value.get("status").and_then(Value::as_str)
                    == Some("component_serialized_conformance")
                    && value.get("live_status").and_then(Value::as_str) == Some("unverified")
            })
}

fn consumer_conformance_matches(consumer: &Value) -> bool {
    consumer.get("contract").and_then(Value::as_str)
        == Some("sts2.protocol/coop-native-v1")
        && consumer.get("protocol_version").and_then(Value::as_str)
            == Some(COOP_NATIVE_PROTOCOL_VERSION)
        && consumer.get("profile").and_then(Value::as_str) == Some(COOP_NATIVE_PROTOCOL_VERSION)
        && consumer.get("schema_digest").and_then(Value::as_str)
            == Some(COOP_NATIVE_SCHEMA_DIGEST)
        && consumer
            .get("provenance")
            .and_then(Value::as_object)
            .is_some_and(|value| {
                value.len() == 3
                    && value.get("schema_source").and_then(Value::as_str)
                        == Some(COOP_NATIVE_SCHEMA_SOURCE)
                    && value.get("generator").and_then(Value::as_str)
                        == Some(COOP_NATIVE_GENERATOR)
                    && value.get("license").and_then(Value::as_str) == Some("MIT")
            })
        && consumer.get("status").and_then(Value::as_str)
            == Some("component_serialized_conformance")
        && consumer.get("source").and_then(Value::as_object).is_some_and(|source| {
            source.get("role").and_then(Value::as_str) == Some("producer")
                && source.get("commit").is_some_and(is_git_identity)
                && source.get("tree").is_some_and(is_git_identity)
                && source.get("result").and_then(Value::as_str) == Some("pass")
                && source.get("live_status").and_then(Value::as_str) == Some("unverified")
        })
        && consumer.get("consumers").and_then(Value::as_array).is_some_and(|values| {
            values.len() == 3
                && values.iter().zip([
                    ("sts2-gateway", "native route and forwarder"),
                    ("sts2-mcp-server", "projection and tool mapping"),
                    ("sts2-harness", "coordination and provider boundary"),
                ]).all(|(value, (name, role))| {
                    value.get("name").and_then(Value::as_str) == Some(name)
                        && value.get("role").and_then(Value::as_str) == Some(role)
                        && value.get("commit").is_some_and(is_git_identity)
                        && value.get("tree").is_some_and(is_git_identity)
                        && value.get("result").and_then(Value::as_str) == Some("pass")
                        && value.get("live_status").and_then(Value::as_str)
                            == Some("unverified")
                })
        })
        && consumer.get("cross_boundary").and_then(Value::as_object).is_some_and(|value| {
            value.get("catalog_action_count") == Some(&Value::from(2))
                && value.get("catalog_vote_count") == Some(&Value::from(1))
                && value.get("source_to_consumer").and_then(Value::as_str) == Some("pass")
        })
        && consumer.get("admission").and_then(Value::as_object).is_some_and(|value| {
            value.get("artifact_status").and_then(Value::as_str)
                == Some("accepted_component")
                && value.get("artifact_admission").and_then(Value::as_str) == Some("component")
        })
}

fn is_git_identity(value: &Value) -> bool {
    value.as_str().is_some_and(|identity| {
        identity.len() == 40
            && identity
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn checksums_match() -> bool {
    let mut files: Vec<(&str, &[u8])> = vec![
        ("../../conformance/cases/coop-native-v1.json", CONFORMANCE),
        ("../../schemas/coop-native-v1.schema.json", SOURCE_SCHEMA),
        ("README.md", README),
        ("conformance.json", CONFORMANCE_COPY),
        ("consumer-conformance.json", CONSUMER_CONFORMANCE),
    ];
    files.extend(GOLDENS);
    files.extend([
        ("manifest.json", MANIFEST),
        ("producer-capture.json", PRODUCER_CAPTURE),
        ("schema.json", SCHEMA),
    ]);

    let mut seen = Vec::with_capacity(files.len());
    for line in CHECKSUMS.lines() {
        let Some((digest, path)) = line.split_once("  ") else {
            return false;
        };
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || seen.contains(&path)
        {
            return false;
        }
        let Some((_, bytes)) = files.iter().find(|(known, _)| *known == path) else {
            return false;
        };
        if digest != sha256(bytes) {
            return false;
        }
        seen.push(path);
    }
    seen.len() == files.len() && files.iter().all(|(path, _)| seen.contains(path))
}

fn sha256(bytes: &[u8]) -> String {
    crate::sha256_hex(bytes)
}
