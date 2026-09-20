// SPDX-License-Identifier: MIT

//! Definition-scoped adoption of an accepted inference-profile revision
//! (#104 acceptance criterion 3, lane 104-B).
//!
//! Everything here is synthetic and labelled as such: the catalogs are
//! in-memory owner doubles, the execution port only records that it was called,
//! and no provider, model, credential or native host is contacted. A pass is
//! source/component evidence for the edit fences, not provider-execution
//! evidence.
//!
//! An admitted run's provenance is written once at admission, so a newer
//! revision is adopted for a *new* definition and the admitted run's own
//! resolution stays exactly what it was.

#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use sts2_harness::management::{
    INFERENCE_PROFILE_PROVENANCE_PREFIX, InferenceProfileRevisionJournal,
    InferenceProfileRevisionResponse, MemoryInferenceProfileRevisionJournal, RevisionAppendOutcome,
    SqliteInferenceProfileRevisionJournal, SqliteWorkflowStore, resolve_definition,
};

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;
#[path = "support/inference_profile_admission_fixtures.rs"]
mod inference_profile_admission_fixtures;
#[path = "support/inference_profile_catalog_doubles.rs"]
mod inference_profile_catalog_doubles;
#[path = "support/inference_profile_catalog_fixtures.rs"]
mod inference_profile_catalog_fixtures;
#[path = "support/inference_profile_revision_support.rs"]
mod inference_profile_revision_support;

use inference_profile_catalog_doubles::*;
use inference_profile_catalog_fixtures::*;
use inference_profile_revision_support::*;

#[test]
fn adoption_changes_a_new_definition_and_never_the_admitted_run()
-> Result<(), Box<dyn std::error::Error>> {
    // Both nodes of the admitted definition pin the exact revision they were
    // authored against, so the owner's later publication of a newer revision
    // cannot substitute another revision for them.
    let catalog = editable_catalog();
    let decide = catalog.descriptors[0].clone();
    let planner = catalog.descriptors[1].clone();
    let pinned = two_node_definition(&reference(&decide), &reference(&planner));
    let journal = Arc::new(MemoryInferenceProfileRevisionJournal::default());
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog.clone()]),
        &pinned,
        "request-admitted",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;
    let admitted = fixture
        .service
        .submit_run(&submitter()?, fixture.request.clone())?;
    let before = persisted_provenance(&fixture, &admitted.workflow_run_id)?;
    assert!(before.starts_with(INFERENCE_PROFILE_PROVENANCE_PREFIX));

    let adopted = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&decide.digest, "mutation-adopt", "1.1.0"),
    )?;
    assert_eq!(adopted.outcome, "adopted");
    assert_eq!(adopted.reference, reference(&adopted.revision));
    assert_ne!(adopted.reference, reference(&decide));

    // The admitted run keeps the exact provenance it resolved at admission,
    // and re-running the execution-side fence's own computation over the
    // admitted definition reproduces it byte for byte.
    let after = persisted_provenance(&fixture, &admitted.workflow_run_id)?;
    assert_eq!(after, before);
    assert_eq!(
        resolve_definition(&catalog, &parsed(&pinned)?, None)?.reference(),
        after
    );

    // The owner is still the only publisher of what it serves: the adopted
    // revision is recorded and named, not spliced into the catalog behind the
    // owner's back, so it cannot be served by any other revision's identity.
    let served = fixture
        .service
        .inference_profile_catalog(&actor(&["workflow:read"])?)?;
    assert_eq!(served.descriptors[0].version, decide.version);
    assert_eq!(served.descriptors[0].digest, decide.digest);
    assert_eq!(
        catalog
            .resolve(&adopted.reference, "decide")
            .err()
            .map(|error| error.code),
        Some("inference_profile_unknown".to_owned())
    );

    // Once the owner publishes the adopted revision, a *new* definition that
    // pins it resolves to exactly that revision and admits — while the run
    // admitted before the edit still names the revision it resolved.
    // The owner publishes both revisions: a catalog may hold several
    // revisions of one profile, and each pinned definition keeps resolving to
    // exactly the revision it named.
    let republished = inference_profile_catalog_fixtures::catalog(vec![
        decide.clone(),
        adopted.revision.clone(),
        planner.clone(),
    ]);
    let new_definition = two_node_definition(&reference(&adopted.revision), &reference(&planner));
    let new_fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![republished.clone()]),
        &new_definition,
        "request-new-definition",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;
    let new_admitted = new_fixture
        .service
        .submit_run(&submitter()?, new_fixture.request.clone())?;
    assert_eq!(new_fixture.submissions.load(Ordering::SeqCst), 1);
    let new_provenance = persisted_provenance(&new_fixture, &new_admitted.workflow_run_id)?;
    assert_ne!(new_provenance, before);
    assert_eq!(
        resolve_definition(&republished, &parsed(&new_definition)?, None)?.reference(),
        new_provenance
    );
    let resolved = resolve_definition(&republished, &parsed(&new_definition)?, None)?;
    assert_eq!(resolved.bindings[0].profile_id, DECIDE);
    assert_eq!(resolved.bindings[0].version, "1.1.0");
    assert_eq!(resolved.bindings[0].digest, adopted.revision.digest);

    // ... and the earlier run keeps its own recorded provenance. The refreshed
    // catalog still resolves that run's pinned revision to the same
    // id/version/digest, but it seals a different catalog identity, so the
    // execution-side fence refuses the admitted definition rather than
    // retargeting it or silently re-sealing it under the newer revision.
    assert_eq!(
        persisted_provenance(&fixture, &admitted.workflow_run_id)?,
        before
    );
    let refreshed = resolve_definition(&republished, &parsed(&pinned)?, None)?;
    assert_ne!(refreshed.reference(), before);
    assert_eq!(refreshed.bindings[0].profile_id, DECIDE);
    assert_eq!(refreshed.bindings[0].version, decide.version);
    assert_eq!(refreshed.bindings[0].digest, decide.digest);
    assert_eq!(journal.history(DECIDE)?.len(), 2);
    Ok(())
}

#[test]
fn the_durable_journal_survives_a_restart() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "sts2-inference-profile-revision-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("store.sqlite3");
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();

    let adopted = {
        let store = Arc::new(SqliteWorkflowStore::open(&path)?);
        let journal: Arc<dyn InferenceProfileRevisionJournal> = Arc::new(
            SqliteInferenceProfileRevisionJournal::new(Arc::clone(&store)),
        );
        let outcome = journal.append(
            DECIDE,
            &served,
            &served.digest,
            "mutation-durable",
            &candidate(&served, "1.1.0")?,
        )?;
        match outcome {
            RevisionAppendOutcome::Adopted(revision) => *revision,
            _ => return Err("the first durable append must be adopted".into()),
        }
    };

    // Reopening the same database keeps the accepted head, the replaced
    // revision and the mutation identity.
    let store = Arc::new(SqliteWorkflowStore::open(&path)?);
    let journal = SqliteInferenceProfileRevisionJournal::new(store);
    let history = journal.history(DECIDE)?;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].digest, served.digest);
    assert_eq!(history[1].digest, adopted.digest);
    assert_eq!(
        journal.head(DECIDE)?.map(|head| head.digest),
        Some(adopted.digest.clone())
    );
    // A second edit against the pre-edit revision now loses the swap.
    let loser = journal.append(
        DECIDE,
        &served,
        &served.digest,
        "mutation-after-restart",
        &candidate(&served, "1.2.0")?,
    )?;
    assert!(matches!(loser, RevisionAppendOutcome::Conflict(_)));
    // The identical retry is replayed from the durable mutation record.
    let replay = journal.append(
        DECIDE,
        &served,
        &served.digest,
        "mutation-durable",
        &candidate(&served, "1.1.0")?,
    )?;
    assert!(matches!(replay, RevisionAppendOutcome::Replayed(_)));
    assert_eq!(journal.history(DECIDE)?.len(), 2);
    Ok(())
}

#[test]
fn both_edit_artifacts_conform_to_the_versioned_closed_schema()
-> Result<(), Box<dyn std::error::Error>> {
    let schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/inference-profile/revision.schema.json"
    ))?;
    let validator = jsonschema::validator_for(&schema)?;
    let catalog_schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/inference-profile/catalog.schema.json"
    ))?;
    let descriptor_schema = json!({
        "$ref": "#/$defs/descriptor",
        "$defs": catalog_schema["$defs"].clone(),
    });
    let descriptor_validator = jsonschema::validator_for(&descriptor_schema)?;

    let request = edit(&"0".repeat(64), "mutation-schema", "1.1.0");
    let encoded = serde_json::to_value(&request)?;
    assert!(
        validator.is_valid(&encoded),
        "the edit request does not conform: {:?}",
        validator
            .iter_errors(&encoded)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
    let mut tampered = encoded.clone();
    tampered["credentials"] = json!("secret");
    assert!(
        !validator.is_valid(&tampered),
        "the closed request schema must reject an unadvertised field"
    );
    let mut floating = encoded.clone();
    floating["version"] = json!("1.1");
    assert!(
        !validator.is_valid(&floating),
        "the closed request schema must reject a non-semver revision"
    );

    let response: InferenceProfileRevisionResponse = adopted_response()?;
    let encoded = serde_json::to_value(&response)?;
    assert!(
        validator.is_valid(&encoded),
        "the edit response does not conform: {:?}",
        validator
            .iter_errors(&encoded)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
    assert!(
        descriptor_validator.is_valid(&encoded["revision"]),
        "the adopted revision is not an inference-profile descriptor"
    );
    let mut stale = encoded.clone();
    stale["revision"]["version"] = json!("1.0");
    assert!(
        !descriptor_validator.is_valid(&stale["revision"]),
        "the descriptor schema must reject a non-semver revision"
    );
    let mut unexplained = encoded.clone();
    unexplained["outcome"] = json!("maybe");
    assert!(
        !validator.is_valid(&unexplained),
        "the response schema must reject an unexplained outcome"
    );
    let mut widened = encoded;
    widened["revision"]["grants"]["edit"] = json!("yes");
    assert!(
        !descriptor_validator.is_valid(&widened["revision"]),
        "the descriptor schema must reject a non-boolean grant"
    );
    Ok(())
}
