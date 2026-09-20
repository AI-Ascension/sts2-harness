// SPDX-License-Identifier: MIT

//! One bridge-owned record per exchange; default records retain their existing shape.

use super::{
    ACTION_QUESTION, Exchange, LIMIT, MAX_PRESENTED_OPTIONS, OptionSelection, RECORD_SCHEMA,
    SelectionMode, build_described_system_one_request, catalog, constraints, decision, framing,
    present, state_with_derived_facts, tactical,
};
use serde_json::{Value, json};

type Failure = Box<dyn std::error::Error>;

pub(super) fn record(
    bytes: &[u8],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    record_profile(bytes, model, gate, exchange, false)
}

pub(super) fn record_profile(
    bytes: &[u8],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
    tactical_enabled: bool,
) -> Result<Value, Failure> {
    if bytes.len() > LIMIT {
        return Err("request exceeds bound".into());
    }
    let request: Value = serde_json::from_slice(bytes)?;
    let catalog = catalog(&request)?;
    if tactical_enabled {
        tactical::validate_catalog(&catalog)?;
    }
    let observation = request
        .get("observation")
        .ok_or("request carries no observation")?;
    let selection = OptionSelection::from_observation(observation, MAX_PRESENTED_OPTIONS);
    if let Some(record) = forced(&selection, &catalog, tactical_enabled)? {
        return Ok(record);
    }
    let options = if tactical_enabled
        && catalog.len() <= tactical::MAX_OPTIONS
        && (selection.presented.is_empty() || selection.presented.len() == catalog.len())
    {
        tactical::catalog_options(observation, &catalog)
    } else {
        present(&selection, observation, &catalog)
    };
    let ids: Vec<String> = options.iter().map(|option| option.id.clone()).collect();
    let body = build_described_system_one_request(
        model,
        &state_with_derived_facts(&request, observation)?,
        &options,
        &framing(
            request["objective"].as_str().unwrap_or_default(),
            observation,
        ),
        &constraints(&request),
    )?;
    exchange_record(body, &ids, catalog.len(), tactical_enabled, gate, exchange)
}

fn forced(
    selection: &OptionSelection,
    catalog: &[String],
    tactical_enabled: bool,
) -> Result<Option<Value>, Failure> {
    if selection.mode != SelectionMode::Forced {
        return Ok(None);
    }
    let Some(only) = selection.presented.first() else {
        return Ok(None);
    };
    if tactical_enabled && !catalog.contains(&only.action_id) {
        return Err("forced action is outside the host catalog".into());
    }
    let mut record = json!({
        "schema": RECORD_SCHEMA, "provider_call": false,
        "provider_request": Value::Null, "provider_response": Value::Null,
        "decision": {
            "decision": "action", "action_id": only.action_id,
            "rationale": "bridge-authored evidence: one legal action, chosen without a provider call",
            "confidence": 100,
        },
    });
    if tactical_enabled {
        record["tactical"] = json!({
            "profile": tactical::PROFILE, "applied": false,
            "fallback_reason": if catalog.len() == 1 { "forced_action" } else { "legacy_forced_group" },
        });
    }
    Ok(Some(record))
}

fn exchange_record(
    mut body: Value,
    ids: &[String],
    catalog_count: usize,
    tactical_enabled: bool,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    let prepared = if tactical_enabled {
        Some(tactical::prepare(body.clone(), catalog_count)?)
    } else {
        None
    };
    if let Some(prepared) = &prepared {
        body = prepared.body.clone();
    }
    let response = exchange(&serde_json::to_vec(&body)?)?;
    if response.len() > LIMIT {
        return Err("provider response exceeds bound".into());
    }
    finish_record(
        body,
        serde_json::from_slice(&response)?,
        prepared,
        ids,
        gate,
    )
}

fn finish_record(
    body: Value,
    response: Value,
    prepared: Option<tactical::Prepared>,
    ids: &[String],
    gate: f64,
) -> Result<Value, Failure> {
    let assessment = if prepared.as_ref().is_some_and(|value| value.applied) {
        Some(tactical::evaluate(&body, &response, gate)?)
    } else {
        None
    };
    let decision = match &assessment {
        Some(assessment) => assessment["decision"].clone(),
        None => decision::map_decision(&response, ACTION_QUESTION, ids, gate)?,
    };
    let mut record = json!({
        "schema": RECORD_SCHEMA, "provider_call": true,
        "provider_request": body, "provider_response": response, "decision": decision,
    });
    if let Some(prepared) = prepared {
        record["tactical"] = json!({
            "profile": tactical::PROFILE, "applied": prepared.applied,
            "fallback_reason": prepared.fallback_reason,
            "question_set_digest": sts2_harness::sha256_hex(serde_json::to_vec(&body["questions"])?),
            "request_digest": sts2_harness::sha256_hex(serde_json::to_vec(&body)?),
            "assessment": assessment,
        });
    }
    Ok(record)
}
