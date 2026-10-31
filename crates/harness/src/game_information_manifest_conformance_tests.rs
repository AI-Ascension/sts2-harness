// SPDX-License-Identifier: MIT
//! Exact accepted protocol vectors, independently checked by the Harness consumer.
use super::*;
use sts2_protocol::GameInformationContentManifestV1Codec;

#[test]
fn copied_artifact_and_every_conformance_vector_match_the_pin() -> TestResult {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let artifact = root.join("protocol-artifact/game-information-content-manifest-v1");
    let sums = std::fs::read_to_string(artifact.join("SHA256SUMS"))?;
    let mut count = 0;
    for line in sums.lines() {
        let (digest, path) = line.split_once("  ").ok_or("checksum")?;
        assert_eq!(
            crate::sha256_hex(std::fs::read(artifact.join(path))?),
            digest,
            "{path}"
        );
        count += 1;
    }
    assert_eq!(count, 5);
    let schema = std::fs::read(artifact.join("schema.json"))?;
    assert_eq!(
        crate::sha256_hex(&schema),
        crate::game_information::manifest::SCHEMA_DIGEST
    );
    assert_eq!(
        schema,
        std::fs::read(root.join("schemas/game-information-content-manifest-v1.schema.json"))?
    );
    let case =
        std::fs::read(root.join("conformance/cases/game-information-content-manifest-v1.json"))?;
    assert_eq!(
        crate::sha256_hex(&case),
        "c9f24a504e892eb509a07a90d33a77d287b14809ae9d2eedca22d402b86bf2ed"
    );
    let codec = GameInformationContentManifestV1Codec::new()?;
    let vectors = [
        (
            "valid/canonical-manifest-response.json",
            "7e9865f6347953f04af109d865fb4133f2ef3b781dc255d4445bc3d91a73b0a0",
            true,
        ),
        (
            "valid/access-denied-error-response.json",
            "7e1e95b4e316962bcd39ab87a91679150b573604de7de6bd55fc62cc18212511",
            true,
        ),
        (
            "invalid/error-with-null-payload.json",
            "70806349122254959c7f3a4af6b9601320f8b19aeae3aa8f7b580ecb85a73371",
            false,
        ),
        (
            "invalid/mismatched-error-code-reason.json",
            "409889730aaf6ef89c3d58b8c6e4d2e31e6350085bc04b2ddbdfa70d099e7d4f",
            false,
        ),
        (
            "invalid/raw-error-reason.json",
            "ee7a263b7062f987be1a975211efcf4c128a09b42451e12d2c59f099607e0bdf",
            false,
        ),
        (
            "invalid/success-with-null-manifest.json",
            "81ea7d8802f975d2414611e5168bdec43b5d09a8945a749c20240871419e2439",
            false,
        ),
        (
            "invalid/unknown-member.json",
            "5053fa2a5eb4e0b42443cd93fe49ccc6b68bbc8c699b66af714f7e88a10008b7",
            false,
        ),
        (
            "invalid/unsupported-schema-digest.json",
            "9266a35c0aab63c5d973c8d83b3b1a773a8250ed056f1380577c660b8727f5b8",
            false,
        ),
    ];
    for (path, digest, valid) in vectors {
        let raw = std::fs::read(
            root.join("conformance/fixtures/game-information-content-manifest-v1")
                .join(path),
        )?;
        assert_eq!(crate::sha256_hex(&raw), digest, "{path}");
        assert_eq!(codec.decode(&raw).is_ok(), valid, "{path}");
        if valid {
            assert_eq!(
                raw,
                std::fs::read(
                    artifact
                        .join("golden")
                        .join(path.strip_prefix("valid/").ok_or("valid vector")?)
                )?
            );
        }
    }
    Ok(())
}

#[test]
fn mcp_mapping_uses_only_owner_context_and_checks_the_complete_result() -> TestResult {
    let source = manifest_value()?;
    let text = serde_json::to_string(&source)?;
    let envelope = json!({"jsonrpc":"2.0","id":42,"result":{
        "content":[{"type":"text","text":text}],"isError":false,"structuredContent":source
    }});
    let context = LookupMcpContext {
        instance_id: "instance".into(),
        mcp_session_id: "mcp".into(),
        lease_id: "lease".into(),
        lease_epoch: 3,
    };
    let raw = call_content_manifest_mcp(&context, 42, |id, request| {
        assert_eq!(id, 42);
        assert_eq!(
            request,
            json!({"name":"sts2.game_information_content_manifest","arguments":{
                "instance_id":"instance","mcp_session_id":"mcp","lease_id":"lease","lease_epoch":3
            }})
        );
        Ok(envelope.clone())
    })?;
    assert_eq!(raw, text.as_bytes());
    for mutation in [
        "id",
        "jsonrpc",
        "rpc_error",
        "isError",
        "multiple",
        "type",
        "structured",
        "correlation",
    ] {
        let mut invalid = envelope.clone();
        match mutation {
            "id" => invalid["id"] = json!(43),
            "jsonrpc" => invalid["jsonrpc"] = json!("3.0"),
            "rpc_error" => invalid["error"] = json!({"code":-1}),
            "isError" => invalid["result"]["isError"] = json!(true),
            "multiple" => {
                invalid["result"]["content"] =
                    json!([{"type":"text","text":text},{"type":"text","text":text}])
            }
            "type" => invalid["result"]["content"][0]["type"] = json!("image"),
            "structured" => {
                invalid["result"]["structuredContent"]["manifest"]["locale"] = json!("fr_FR")
            }
            "correlation" => {
                invalid["result"]["content"][0]["text"] = json!(text.replace("\"42\"", "\"43\""))
            }
            _ => return Err("fixture".into()),
        }
        assert!(
            content_manifest_response_mcp(&invalid, 42).is_err(),
            "{mutation}"
        );
    }
    assert!(content_manifest_response_mcp(&envelope, 0).is_err());
    Ok(())
}

#[test]
fn every_closed_refusal_mapping_reaches_the_consumer_without_raw_diagnostics() -> TestResult {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../conformance/cases/game-information-content-manifest-v1.json"
    ))?;
    for error in cases["error_mapping"]
        .as_object()
        .ok_or("error mappings")?
        .values()
    {
        let mut value = manifest_value()?;
        value["kind"] = json!("error_response");
        value["manifest"] = Value::Null;
        value["error"] = error.clone();
        let text = serde_json::to_string(&value)?;
        let envelope = json!({"jsonrpc":"2.0","id":42,"result":{
            "content":[{"type":"text","text":text}],"isError":true
        }});
        let raw = content_manifest_response_mcp(&envelope, 42)?;
        let (mut session, mut corpus) = manifest_session(8192)?;
        let delivery = read(&mut session, &mut corpus, raw)?;
        assert_eq!(delivery.data["error"], *error);
        assert!(delivery.data["manifest"].is_null());
        value["error"]["reason"] = json!("private/path/diagnostic");
        assert!(
            read(
                &mut manifest_session(8192)?.0,
                &mut corpus,
                serde_json::to_vec(&value)?
            )
            .is_err()
        );
    }
    Ok(())
}
