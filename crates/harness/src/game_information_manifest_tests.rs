// SPDX-License-Identifier: MIT
//! Synthetic whole-catalog admission, identity, retention and budget regressions.
use super::*;
use crate::game_information::manifest::{ContentManifestDelivery, MAX_RESPONSE_BYTES};

#[path = "game_information_manifest_archive_tests.rs"]
mod archive_tests;
#[path = "game_information_manifest_conformance_tests.rs"]
mod conformance_tests;
#[cfg(unix)]
#[path = "game_information_manifest_process_tests.rs"]
mod process_tests;

fn manifest_value() -> Result<Value, serde_json::Error> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../protocol-artifact/game-information-content-manifest-v1/golden/canonical-manifest-response.json"
    ))?;
    value["correlation_id"] = json!("42");
    Ok(value)
}

fn manifest_session(
    budget: usize,
) -> Result<(LookupSession, MemoryCorpus), Box<dyn std::error::Error>> {
    let (mut session, corpus) = setup(budget)?;
    session.binding.content_manifest_id = "c".repeat(64);
    session.binding.locale = "en_US".to_owned();
    negotiate(&mut session)?;
    Ok((session, corpus))
}

struct ManifestPort {
    raw: Vec<u8>,
    calls: usize,
}
impl LookupMcpPort for ManifestPort {
    fn information_correlation(&self) -> Result<String, LookupError> {
        Ok("42".to_owned())
    }
    fn call_information(&mut self, _: &str, _: &Value) -> Result<Vec<u8>, LookupError> {
        Err(LookupError::Divergence)
    }
    fn information_content_manifest(&mut self) -> Result<(String, Vec<u8>), LookupError> {
        self.calls += 1;
        Ok(("42".to_owned(), self.raw.clone()))
    }
}
fn read(
    session: &mut LookupSession,
    corpus: &mut MemoryCorpus,
    raw: Vec<u8>,
) -> Result<ContentManifestDelivery, LookupError> {
    let binding = session.binding.clone();
    session.content_manifest_port(
        &binding,
        "manifest-1",
        corpus,
        &mut ManifestPort { raw, calls: 0 },
    )
}

fn large_manifest() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut value = manifest_value()?;
    let definition = value["manifest"]["definitions"][0].clone();
    let definitions = (0..240)
        .map(|index| {
            let mut definition = definition.clone();
            definition["namespaced_id"] = json!(format!("synthetic:card_{index:03}"));
            definition
        })
        .collect::<Vec<_>>();
    value["manifest"]["definitions"] = json!(definitions);
    value["manifest"]["families"][0]["definition_count"] = json!(240);
    let raw = serde_json::to_vec_pretty(&value)?;
    assert!(raw.len() > MAX_SOURCE_BYTES && raw.len() < MAX_RESPONSE_BYTES);
    Ok(raw)
}

#[test]
fn complete_catalog_and_typed_refusal_remain_distinct_data() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let value = manifest_value()?;
    let full = read(&mut session, &mut corpus, serde_json::to_vec(&value)?)?;
    assert_eq!(full.data["manifest"], value["manifest"]);
    assert_eq!(full.data["authority"], "untrusted_game_information_data");
    assert!(full.data["error"].is_null());
    assert!(session.binding.snapshot.is_none());
    let mut denied: Value = serde_json::from_str(include_str!(
        "../../../protocol-artifact/game-information-content-manifest-v1/golden/access-denied-error-response.json"
    ))?;
    denied["correlation_id"] = json!("42");
    let (mut session, mut corpus) = manifest_session(8192)?;
    let delivery = read(&mut session, &mut corpus, serde_json::to_vec(&denied)?)?;
    assert!(delivery.data["manifest"].is_null());
    assert_eq!(delivery.data["error"], denied["error"]);
    assert!(
        delivery.record.error.is_none(),
        "a typed producer refusal is retained source data"
    );
    Ok(())
}

#[test]
fn whole_transport_bound_retains_two_chunks_and_exact_all_bytes() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let mut raw = large_manifest()?;
    raw.resize(MAX_RESPONSE_BYTES, b' ');
    let delivery = read(&mut session, &mut corpus, raw.clone())?;
    assert_eq!(delivery.record.sources.len(), 2);
    assert_eq!(delivery.data["delivery"], "retained");
    assert!(
        delivery.data.get("manifest").is_none(),
        "no shortened catalog is delivered"
    );
    let mut reconstructed = Vec::new();
    while reconstructed.len() < raw.len() {
        let chunk =
            session.read_retained_manifest(&delivery.record, &corpus, reconstructed.len())?;
        assert!(!chunk.is_empty() && chunk.len() <= 8192);
        reconstructed.extend(chunk);
    }
    assert_eq!(reconstructed, raw);
    assert_eq!(delivery.record.source_sha256, Some(crate::sha256_hex(&raw)));
    assert_eq!(
        session.read_retained_manifest(&delivery.record, &corpus, raw.len())?,
        Vec::<u8>::new()
    );
    assert_eq!(
        session.read_retained_manifest(&delivery.record, &corpus, raw.len() + 1),
        Err(LookupError::Bounds)
    );
    let mut reordered = delivery.record;
    reordered.sources.reverse();
    assert!(matches!(
        session.replay_content_manifest(&reordered, &corpus),
        Err(LookupError::Divergence)
    ));
    Ok(())
}

#[test]
fn capacity_and_model_budget_failures_publish_no_partial_source() -> TestResult {
    let (mut session, corpus) = manifest_session(8192)?;
    let mut small = MemoryCorpus::with_limits(corpus.scope().clone(), 10_000, 70_000)?;
    for entry in corpus.entries() {
        small.admit(entry.clone())?;
    }
    let before = (
        small.total_bytes(),
        small.generation(),
        small.entries().count(),
    );
    assert_eq!(
        read(&mut session, &mut small, large_manifest()?),
        Err(LookupError::Retention)
    );
    assert_eq!(
        before,
        (
            small.total_bytes(),
            small.generation(),
            small.entries().count()
        )
    );
    assert!(session.manifest_records[0].sources.is_empty());
    let (mut session, mut corpus) = manifest_session(1)?;
    let before = (corpus.total_bytes(), corpus.generation());
    assert_eq!(
        read(&mut session, &mut corpus, large_manifest()?),
        Err(LookupError::Bounds)
    );
    assert_eq!(before, (corpus.total_bytes(), corpus.generation()));
    Ok(())
}

#[test]
fn duplicate_sources_keep_original_retention_and_require_current_readability() -> TestResult {
    let (mut original, mut corpus) = manifest_session(8192)?;
    let raw = serde_json::to_vec(&manifest_value()?)?;
    let delivery = read(&mut original, &mut corpus, raw.clone())?;
    let before = (
        corpus.total_bytes(),
        corpus.generation(),
        corpus.entries().cloned().collect::<Vec<_>>(),
    );
    for (now, readable) in [
        (NOW, true),
        ("2026-09-15T12:00:00Z", true),
        ("2026-09-14T12:00:00Z", false),
        ("2026-09-14T23:59:59Z", false),
        ("2026-09-15T23:59:59Z", true),
        (EXPIRY, false),
        ("2026-09-17T12:00:00Z", false),
    ] {
        let mut fresh = LookupSession::new(
            original.binding.clone(),
            original.policy.clone(),
            &corpus,
            now,
            "2026-09-19T00:00:00Z",
        )?;
        negotiate(&mut fresh)?;
        let result = read(&mut fresh, &mut corpus, raw.clone());
        if readable {
            let repeated = result?;
            assert_eq!(repeated.record.sources, delivery.record.sources);
            assert_eq!(
                fresh.replay_content_manifest(&repeated.record, &corpus)?,
                repeated
            );
        } else {
            assert_eq!(result, Err(LookupError::MissingRetention), "{now}");
            let failure = &fresh.manifest_records[0];
            assert_eq!(failure.error, Some(LookupError::MissingRetention));
            assert!(failure.sources.is_empty());
            assert!(failure.source_sha256.is_none() && failure.correlation_id.is_none());
            assert_eq!(failure.source_bytes, 0);
        }
        assert_eq!(
            before,
            (
                corpus.total_bytes(),
                corpus.generation(),
                corpus.entries().cloned().collect::<Vec<_>>()
            )
        );
    }
    Ok(())
}

#[test]
fn scope_invalidation_and_duplicate_operation_refuse_before_mcp() -> TestResult {
    let (mut session, mut corpus) = manifest_session(8192)?;
    let mut port = ManifestPort {
        raw: serde_json::to_vec(&manifest_value()?)?,
        calls: 0,
    };
    let mut wrong = session.binding.clone();
    wrong.authority_epoch += 1;
    assert_eq!(
        session.content_manifest_port(&wrong, "manifest-1", &mut corpus, &mut port),
        Err(LookupError::Scope)
    );
    assert_eq!(port.calls, 0);
    let binding = session.binding.clone();
    session.content_manifest_port(&binding, "manifest-1", &mut corpus, &mut port)?;
    assert_eq!(
        session.content_manifest_port(&binding, "manifest-1", &mut corpus, &mut port),
        Err(LookupError::Divergence)
    );
    session.invalidate();
    assert_eq!(
        session.content_manifest_port(&binding, "manifest-2", &mut corpus, &mut port),
        Err(LookupError::MissingCapability)
    );
    assert_eq!(port.calls, 1);
    Ok(())
}

#[test]
fn catalog_identity_accounting_and_unknown_origin_are_validated() -> TestResult {
    let original = manifest_value()?;
    for mutation in [
        "binding",
        "locale",
        "correlation",
        "count",
        "duplicate",
        "family",
        "order",
        "origin",
        "version",
        "override",
    ] {
        let mut value = original.clone();
        match mutation {
            "binding" => value["manifest"]["inventory_revision"] = json!("d".repeat(64)),
            "locale" => value["manifest"]["locale"] = json!("fr_FR"),
            "correlation" => value["correlation_id"] = json!("43"),
            "count" => value["manifest"]["families"][0]["definition_count"] = json!(2),
            "duplicate" => {
                value["manifest"]["definitions"] = json!([
                    value["manifest"]["definitions"][0],
                    value["manifest"]["definitions"][0]
                ])
            }
            "family" => value["manifest"]["families"] = json!([]),
            "order" => {
                value["manifest"]["packages"] = json!([
                    value["manifest"]["packages"][0],
                    value["manifest"]["packages"][0]
                ])
            }
            "origin" => {
                value["manifest"]["definitions"][0]["origin"]["package_id"] = json!("unknown")
            }
            "version" => {
                value["manifest"]["definitions"][0]["origin"]["package_version"] =
                    json!("different")
            }
            "override" => {
                value["manifest"]["definitions"][0]["override_chain"] = json!(["base", "base"])
            }
            _ => return Err("unhandled fixture".into()),
        }
        let (mut session, mut corpus) = manifest_session(8192)?;
        let before = corpus.total_bytes();
        assert!(
            read(&mut session, &mut corpus, serde_json::to_vec(&value)?).is_err(),
            "{mutation}"
        );
        assert_eq!(corpus.total_bytes(), before);
    }
    let mut unknown = original;
    unknown["manifest"]["packages"][0]["order"] = json!(7); // Orders need not be contiguous.
    unknown["manifest"]["definitions"][0]["origin"] =
        json!({"package_id":null,"package_version":null});
    unknown["manifest"]["definitions"][0]["override_chain"] = json!([]);
    unknown["manifest"]["definitions"][0]["handled"] = json!(false);
    unknown["manifest"]["families"][0]["handled"] = json!(false);
    let (mut session, mut corpus) = manifest_session(8192)?;
    let delivery = read(&mut session, &mut corpus, serde_json::to_vec(&unknown)?)?;
    assert_eq!(delivery.data["manifest"], unknown["manifest"]);
    Ok(())
}

#[test]
fn oversize_duplicate_json_wrong_profile_and_digest_are_not_retained() -> TestResult {
    let value = manifest_value()?;
    let raw = serde_json::to_string(&value)?;
    let cases = [
        vec![b' '; MAX_RESPONSE_BYTES + 1],
        raw.replacen(
            "\"game_build\":",
            "\"game_build\":\"duplicate\",\"game_build\":",
            1,
        )
        .into_bytes(),
        raw.replace(
            crate::game_information::manifest::SCHEMA_DIGEST,
            &"0".repeat(64),
        )
        .into_bytes(),
        raw.replace(
            crate::game_information::manifest::PROFILE,
            "game-information-query-v1",
        )
        .into_bytes(),
    ];
    for raw in cases {
        let (mut session, mut corpus) = manifest_session(8192)?;
        assert!(read(&mut session, &mut corpus, raw).is_err());
        assert_eq!(corpus.entries().count(), 1);
        assert!(session.manifest_records[0].source_sha256.is_none());
    }
    Ok(())
}
