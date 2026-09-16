// SPDX-License-Identifier: MIT

use super::exo_contract_capability::execute_capability_vectors;
use super::exo_contract_conformance::{
    execute_decision_vectors, execute_envelope_vectors, execute_request_vectors,
};
use super::*;
use serde_json::Value;

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
