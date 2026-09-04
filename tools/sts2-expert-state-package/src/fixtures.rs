// SPDX-License-Identifier: MIT

//! Checks requirements fixture envelopes, never production runtime execution.
use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;

use crate::integrity::{ids, read_json, require, strings};
use crate::required_string;
use serde_json::Value;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub(crate) fn validate_fixtures(root: &Path, states: &[Value]) -> Result<()> {
    let manifest = read_json(&root.join("fixtures/manifest.json"))?;
    let expected = ids(states, "state_id")?;
    require(
        strings(&manifest["candidate_state_ids"])? == expected,
        "fixture registry drift",
    )?;
    let classes = manifest["fixture_classes"]
        .as_array()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing fixture classes"))?;
    let mut all_ids = BTreeSet::new();
    for class in classes {
        let name = required_string(class, "name")?;
        let path = required_string(class, "path")?;
        require(
            Path::new(path).file_name().and_then(|name| name.to_str()) == Some(path),
            "unsafe fixture path",
        )?;
        let rows = fs::read_to_string(root.join("fixtures").join(path))?
            .lines()
            .map(serde_json::from_str)
            .collect::<std::result::Result<Vec<Value>, _>>()?;
        require(
            ids(&rows, "state_id")? == expected,
            "fixture state coverage drift",
        )?;
        require(
            class["records"].as_u64() == Some(rows.len() as u64),
            "fixture count drift",
        )?;
        for row in &rows {
            require(
                all_ids.insert(required_string(row, "fixture_id")?.to_owned()),
                "duplicate fixture ID",
            )?;
            require(row["fixture_class"] == name, "fixture class drift")?;
            require(
                row["provenance"]["source_sha256"] == manifest["source"]["sha256"],
                "fixture source digest drift",
            )?;
            validate_fixture(row)?;
            let assertions = strings(&row["assertions"])?;
            require(
                strings(&manifest["required_assertions"])?.is_subset(&assertions)
                    && strings(&manifest["class_required_assertions"][name])?
                        .is_subset(&assertions),
                "fixture assertion declaration missing",
            )?;
        }
    }
    require(
        manifest["expected_records"].as_u64() == Some(all_ids.len() as u64),
        "fixture total drift",
    )
}

pub(crate) fn validate_fixture(row: &Value) -> Result<()> {
    require(
        row["evidence_status"] == "proposed"
            && row["provenance"]["kind"] == "synthetic"
            && row["provenance"]["live_capture"] == false
            && row["provenance"]["claims_live_observation"] == false,
        "fixture provenance upgraded",
    )?;
    let observation = &row["normalized_observation"];
    require(
        observation["state_id"] == row["state_id"]
            && observation["privileged_fields"] == serde_json::json!([])
            && observation["generation"]
                .as_u64()
                .is_some_and(|value| value > 0),
        "invalid fixture observation",
    )?;
    let actions = row["legal_actions"]
        .as_array()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing fixture actions"))?;
    for action in actions {
        require(
            action["mutates_state"] == false
                && action["generated_from_observation_id"] == observation["observation_id"]
                && action["generated_from_generation"] == observation["generation"]
                && action["observation_generation"] == observation["generation"],
            "fixture action generation or mutation mismatch",
        )?;
        require(
            strings(&action["target_domain"]["selected_ids"])?
                .is_subset(&strings(&action["target_domain"]["allowed_ids"])?),
            "fixture target outside domain",
        )?;
    }
    require(
        row["patch_control"]["autonomous_mutation_allowed"] == false,
        "fixture patch quarantine removed",
    )?;
    let behavior = &row["expected_behavior"];
    require(
        behavior["unknown_state"]
            .as_str()
            .is_some_and(|value| value.starts_with("fail_closed"))
            && behavior["stale_observation"] == "reject_action_and_reobserve"
            && behavior["delayed_or_uncertain_mutation"] == "reconcile_read_only_before_any_retry"
            && behavior["build_mismatch"] == "patch_quarantine_before_autonomous_mutation",
        "fixture required recovery behavior changed",
    )?;
    match row["fixture_class"].as_str() {
        Some("adversarial") => require(
            observation["stale"] == true && row["test_only_injections"].is_object(),
            "missing adversarial condition",
        )?,
        Some("recovery") => require(
            observation["stale"] == true
                && row["execution"]["mutation_status"] == "unknown"
                && row["execution"]["settlement"] == "delayed_or_uncertain",
            "missing uncertain recovery condition",
        )?,
        Some("patch_regression") => require(
            row["patch_control"]["migration_status"] == "quarantined"
                && row["patch_regression"]["candidate_fixture_is_not_promoted"] == true,
            "missing patch quarantine condition",
        )?,
        Some("normal" | "boundary") => {}
        _ => return require(false, "unknown fixture class"),
    }
    Ok(())
}
