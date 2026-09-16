// SPDX-License-Identifier: MIT

use super::exo_contract_capability::execute_capability_vectors;
use super::exo_contract_conformance::{
    execute_decision_vectors, execute_envelope_vectors, execute_request_vectors,
};
use super::*;
use serde_json::Value;

const REQUIRED_AC4_MEMBERS: &[(&str, &str, &[&str])] = &[
    (
        "semantic_decision_variant",
        "decision_vectors",
        &[
            "plan",
            "action",
            "wait",
            "reobserve",
            "recovery_reobserve",
            "recovery_reconcile",
            "recovery_release_lease",
            "recovery_stop_episode",
            "unknown_decision_kind",
        ],
    ),
    (
        "ordinary_and_map_request_bound",
        "request_vectors",
        &[
            "standard_at_bound",
            "standard_over_bound",
            "map_at_bound",
            "map_over_bound",
            "envelope_overhead_standard",
            "envelope_overhead_map",
            "map_request_over_ordinary_bound",
        ],
    ),
    (
        "wrong_correlation",
        "envelope_vectors",
        &[
            "wrong_request_id",
            "wrong_turn_id",
            "wrong_control_identity",
        ],
    ),
    (
        "incompatible_schema_version",
        "request_vectors",
        &["wrong_schema", "map_wrong_schema"],
    ),
    (
        "incompatible_schema_version",
        "envelope_vectors",
        &["wrong_wire_version"],
    ),
    (
        "incompatible_schema_version",
        "capability_vectors",
        &["capability_schema_version", "capability_contract_version"],
    ),
    (
        "explicitly_unavailable_capability",
        "capability_vectors",
        &[
            "terminal_decision",
            "turn_identity",
            "graceful_eof",
            "idempotency",
            "cancellation",
            "recovery",
            "profile_map",
            "profile_expert",
            "context_continuity",
            "profile_standard",
        ],
    ),
];

#[test]
fn ac4_vector_corpus_index_enumerates_every_class_and_is_consumed() {
    let conformance: Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    let index = conformance
        .get("corpus_index")
        .expect("conformance vector corpus declares an index")
        .clone();
    assert_eq!(
        index["corpus_index_schema"].as_str(),
        Some("sts2.exo-bridge-corpus-index-v1"),
        "corpus index schema marker"
    );
    assert_eq!(
        index["contract_version"].as_str(),
        Some("sts2-exo-bridge-v1"),
        "corpus index contract version"
    );
    assert_eq!(
        verify_required_ac4_membership(&conformance, &index),
        Ok(()),
        "the corpus and index retain the independently pinned AC4 vectors"
    );
    let names = |key: &str| -> Vec<String> {
        conformance[key]
            .as_array()
            .unwrap_or_else(|| unreachable!("{key} is an array"))
            .iter()
            .map(|vector| {
                vector["name"]
                    .as_str()
                    .expect("conformance vector name")
                    .to_owned()
            })
            .collect()
    };
    let request = names("request_vectors");
    let decision = names("decision_vectors");
    let envelope = names("envelope_vectors");
    let capability = names("capability_vectors");
    let mut corpus: Vec<String> = [
        request.clone(),
        decision.clone(),
        envelope.clone(),
        capability.clone(),
    ]
    .concat();
    corpus.sort();

    let consumed: Vec<String> = [
        execute_request_vectors(
            conformance["request_vectors"]
                .as_array()
                .expect("request vectors"),
        ),
        execute_decision_vectors(
            conformance["decision_vectors"]
                .as_array()
                .expect("decision vectors"),
        ),
        execute_envelope_vectors(
            conformance["envelope_vectors"]
                .as_array()
                .expect("envelope vectors"),
        ),
        execute_capability_vectors(
            conformance["capability_vectors"]
                .as_array()
                .expect("capability vectors"),
        ),
    ]
    .concat();
    let mut consumed_sorted = consumed.clone();
    consumed_sorted.sort();
    consumed_sorted.dedup();
    assert_eq!(
        consumed_sorted.len(),
        consumed.len(),
        "a corpus vector was consumed more than once"
    );
    assert_eq!(
        consumed_sorted, corpus,
        "a corpus vector was not consumed by the production validator"
    );

    let classes = index["classes"].as_array().expect("corpus index classes");
    assert!(!classes.is_empty(), "corpus index declares no classes");
    let mut indexed: Vec<String> = Vec::new();
    let mut ac4_classes: Vec<String> = Vec::new();
    let mut totals = std::collections::BTreeMap::new();
    for class in classes {
        let class_name = class["class"].as_str().expect("corpus index class name");
        assert!(
            class
                .get("acceptance")
                .and_then(Value::as_str)
                .is_none_or(|acceptance| !acceptance.is_empty()),
            "{class_name} has an empty acceptance marker"
        );
        if class["acceptance"].as_str() == Some("issue-139-ac4") {
            ac4_classes.push(class_name.to_owned());
        }
        let mut members = 0_usize;
        for source in class["sources"].as_array().expect("class sources") {
            let key = source["source"].as_str().expect("class source key");
            let available = match key {
                "request_vectors" => &request,
                "decision_vectors" => &decision,
                "envelope_vectors" => &envelope,
                "capability_vectors" => &capability,
                other => unreachable!("unknown corpus source {other}"),
            };
            let declared: Vec<String> = source["vectors"]
                .as_array()
                .expect("class source vectors")
                .iter()
                .map(|vector| vector.as_str().expect("class vector name").to_owned())
                .collect();
            assert!(
                !declared.is_empty(),
                "{class_name}/{key} declares no vectors"
            );
            let filtered: Vec<String> = available
                .iter()
                .filter(|name| declared.contains(name))
                .cloned()
                .collect();
            assert_eq!(
                filtered, declared,
                "{class_name}/{key} index members do not match the corpus order"
            );
            members += declared.len();
            indexed.extend(declared);
        }
        assert!(members > 0, "{class_name} indexes no corpus vector");
        totals.insert(class_name.to_owned(), members);
    }
    let mut indexed_sorted = indexed.clone();
    indexed_sorted.sort();
    assert_eq!(
        indexed_sorted, corpus,
        "the corpus index must enumerate every conformance vector exactly once"
    );
    for required in [
        "semantic_decision_variant",
        "ordinary_and_map_request_bound",
        "wrong_correlation",
        "incompatible_schema_version",
        "explicitly_unavailable_capability",
    ] {
        assert!(
            ac4_classes.iter().any(|class| class == required),
            "AC4 class is absent from the corpus index: {required}"
        );
    }
    let declared_totals = index["totals"]
        .as_object()
        .expect("corpus index totals are an object");
    for (key, corpus_names) in [
        ("request_vectors", &request),
        ("decision_vectors", &decision),
        ("envelope_vectors", &envelope),
        ("capability_vectors", &capability),
    ] {
        assert_eq!(
            declared_totals[key].as_u64(),
            Some(corpus_names.len() as u64),
            "{key} total count"
        );
    }
}

#[test]
fn ac4_membership_witness_rejects_synchronized_vector_removal() {
    let mut conformance: Value =
        serde_json::from_slice(CONFORMANCE).expect("conformance vectors are valid JSON");
    let decision_count = {
        let decisions = conformance["decision_vectors"]
            .as_array_mut()
            .expect("decision vectors");
        decisions.retain(|vector| vector["name"].as_str() != Some("unknown_decision_kind"));
        decisions.len()
    };
    let classes = conformance["corpus_index"]["classes"]
        .as_array_mut()
        .expect("corpus index classes");
    let semantic = classes
        .iter_mut()
        .find(|class| class["class"].as_str() == Some("semantic_decision_variant"))
        .expect("semantic decision class");
    let decision_source = semantic["sources"]
        .as_array_mut()
        .expect("semantic decision sources")
        .iter_mut()
        .find(|source| source["source"].as_str() == Some("decision_vectors"))
        .expect("semantic decision vector source");
    decision_source["vectors"]
        .as_array_mut()
        .expect("indexed decision vectors")
        .retain(|vector| vector.as_str() != Some("unknown_decision_kind"));
    conformance["corpus_index"]["totals"]["decision_vectors"] = serde_json::json!(decision_count);

    let index = conformance["corpus_index"].clone();
    assert!(
        verify_required_ac4_membership(&conformance, &index).is_err(),
        "removing an AC4 vector from both the corpus and index must fail"
    );
}

fn verify_required_ac4_membership(conformance: &Value, index: &Value) -> Result<(), String> {
    let classes = index
        .get("classes")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("corpus index classes are missing"))?;
    for (class_name, source_name, required_vectors) in REQUIRED_AC4_MEMBERS {
        let class = classes
            .iter()
            .find(|class| {
                class.get("class").and_then(Value::as_str) == Some(*class_name)
                    && class.get("acceptance").and_then(Value::as_str) == Some("issue-139-ac4")
            })
            .ok_or_else(|| format!("required AC4 class is missing: {class_name}"))?;
        let source = class
            .get("sources")
            .and_then(Value::as_array)
            .and_then(|sources| {
                sources.iter().find(|source| {
                    source.get("source").and_then(Value::as_str) == Some(*source_name)
                })
            })
            .ok_or_else(|| format!("{class_name} lacks source {source_name}"))?;
        let indexed_vectors = source
            .get("vectors")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{class_name}/{source_name} vectors are missing"))?;
        let corpus_vectors = conformance
            .get(*source_name)
            .and_then(Value::as_array)
            .ok_or_else(|| format!("corpus source is missing: {source_name}"))?;
        for required in *required_vectors {
            if !corpus_vectors
                .iter()
                .any(|vector| vector["name"].as_str() == Some(required))
            {
                return Err(format!(
                    "{class_name}/{source_name} lacks required corpus vector {required}"
                ));
            }
            if !indexed_vectors
                .iter()
                .any(|vector| vector.as_str() == Some(required))
            {
                return Err(format!(
                    "{class_name}/{source_name} does not index required vector {required}"
                ));
            }
        }
    }
    Ok(())
}
