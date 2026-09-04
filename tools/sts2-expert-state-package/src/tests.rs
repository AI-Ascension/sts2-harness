// SPDX-License-Identifier: MIT

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{
    integrity, load_records, render_state_diagram, render_state_report, validate_state_ids,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/research/sts2-expert-state-package")
}

#[test]
fn every_inventory_reference_digest_and_fixture_is_checked() -> Result<(), Box<dyn Error>> {
    let root = root();
    let states = load_records(&root, "states.json")?;
    validate_state_ids(&states)?;
    for (name, expected) in [
        ("observations.json", 4315),
        ("actions.json", 421),
        ("transitions.json", 1059),
    ] {
        crate::validate_record_count(&root, name, expected)?;
    }
    integrity::validate(&root, &states)
}

#[test]
fn every_generated_report_and_diagram_matches_current_renderer() -> Result<(), Box<dyn Error>> {
    let root = root();
    let states = load_records(&root, "states.json")?;
    for state in &states {
        let id = crate::required_string(state, "state_id")?;
        assert_eq!(
            fs::read_to_string(root.join(format!("report/{id}.md")))?,
            render_state_report(state)?
        );
        assert_eq!(
            fs::read_to_string(root.join(format!("diagrams/states/{id}.mmd")))?,
            render_state_diagram(state)?
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("diagrams/global-architecture.mmd"))?,
        crate::global_architecture()
    );
    assert_eq!(
        fs::read_to_string(root.join("diagrams/decision-policy.mmd"))?,
        crate::decision_policy()
    );
    Ok(())
}

#[test]
fn dangling_and_cross_state_references_fail_even_when_counts_match() -> Result<(), Box<dyn Error>> {
    let root = root();
    let states = load_records(&root, "states.json")?;
    let observations = load_records(&root, "observations.json")?;
    let actions = load_records(&root, "actions.json")?;
    let transitions = load_records(&root, "transitions.json")?;
    let mut changed = transitions.clone();
    changed[0]["target_state_id"] = json!("not_a_state");
    assert!(integrity::validate_joins(&states, &observations, &actions, &changed).is_err());
    changed = transitions.clone();
    changed[0]["action_id"] = json!("action:title_screen:a01");
    assert!(integrity::validate_joins(&states, &observations, &actions, &changed).is_err());
    let mut changed_states = states.clone();
    changed_states[0]["legal_actions"] = json!([]);
    assert!(
        integrity::validate_joins(&changed_states, &observations, &actions, &transitions).is_err()
    );
    let mut changed_fields = observations.clone();
    for field in &mut changed_fields {
        if field["access_class"] == "PRIVILEGED" {
            field["production_observation"] = json!(true);
            break;
        }
    }
    assert!(integrity::validate_joins(&states, &changed_fields, &actions, &transitions).is_err());
    Ok(())
}

#[test]
fn unsafe_or_colliding_output_names_are_rejected() -> Result<(), Box<dyn Error>> {
    let mut states = load_records(&root(), "states.json")?;
    states[0]["state_id"] = json!("../outside");
    assert!(validate_state_ids(&states).is_err());
    states[0]["state_id"] = json!("title_screen");
    assert!(validate_state_ids(&states).is_err());
    Ok(())
}

#[test]
fn recovery_diagrams_have_bounded_terminal_paths_without_redispatch() -> Result<(), Box<dyn Error>>
{
    let states = load_records(&root(), "states.json")?;
    let first = states.first().ok_or("empty state inventory")?;
    for diagram in [
        render_state_diagram(first)?,
        crate::decision_policy().to_owned(),
    ] {
        assert!(diagram.contains("shared non-resetting monotonic deadline and attempt budget"));
        assert!(diagram.contains("C -->|no| H"));
        assert!(diagram.contains("C -->|yes| R"));
        assert!(diagram.contains("R --> K"));
        assert!(diagram.contains("operation-bound authoritative outcome known?"));
        assert!(diagram.contains("K -->|yes| M"));
        assert!(diagram.contains("K -->|uncertain| C"));
        for forbidden in ["H -->", "R --> B", "R --> Q", "K -->|yes| B", "M -->"] {
            assert!(
                !diagram.contains(forbidden),
                "unexpected recovery edge {forbidden}"
            );
        }
    }
    assert!(crate::decision_policy().contains("O --> Q"));
    Ok(())
}

#[test]
fn forged_fixture_targets_generation_mutation_and_evidence_are_rejected()
-> Result<(), Box<dyn Error>> {
    let text = fs::read_to_string(root().join("fixtures/normal.jsonl"))?;
    let first = text.lines().next().ok_or("empty fixture file")?;
    let fixture: Value = serde_json::from_str(first)?;
    for (pointer, value) in [
        ("/legal_actions/0/generated_from_generation", json!(2)),
        (
            "/legal_actions/0/target_domain/selected_ids",
            json!(["forged"]),
        ),
        ("/legal_actions/0/mutates_state", json!(true)),
        (
            "/normalized_observation/privileged_fields",
            json!(["hidden_rng"]),
        ),
        ("/provenance/live_capture", json!(true)),
        ("/patch_control/autonomous_mutation_allowed", json!(true)),
    ] {
        let mut changed = fixture.clone();
        *changed
            .pointer_mut(pointer)
            .ok_or("missing fixture field")? = value;
        assert!(
            crate::fixtures::validate_fixture(&changed).is_err(),
            "{pointer}"
        );
    }
    Ok(())
}
