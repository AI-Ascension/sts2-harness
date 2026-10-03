// SPDX-License-Identifier: MIT

//! One bridge-owned record per exchange; default records retain their existing shape.

use super::{
    ACTION_QUESTION, CARD_CHOICE_QUESTION, Exchange, KIND_QUESTION, LIMIT, MAX_PRESENTED_OPTIONS,
    OptionSelection, RECORD_SCHEMA, SelectionMode, ask, build_class_system_one_request,
    build_described_system_one_request, catalog, constraints, decision, framing, present,
    state_with_derived_facts, tactical,
};
use serde_json::{Value, json};
use sts2_harness::DisclosedCardChoice;

type Failure = Box<dyn std::error::Error>;

pub(super) fn record(
    bytes: &[u8],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    record_profile(bytes, model, gate, exchange, false, true)
}

/// Records one decision, shaping the ask to what the caller's profile permits.
///
/// `tactical_enabled` selects the evaluation profile, which owns its own question-count contract.
/// `two_stage_allowed` is `false` for a caller that permits only one transport invocation (the
/// capture profile), so an above-bound set is asked as one question there rather than split.
pub(super) fn record_profile(
    bytes: &[u8],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
    tactical_enabled: bool,
    two_stage_allowed: bool,
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
    // A presented set above the option bound is asked in two stages when more than one exchange is
    // allowed. The evaluation profile owns its own question-count contract, and the capture profile
    // permits at most one transport invocation, so each keeps the single question it was reviewed
    // with; the split is otherwise the production path's behaviour.
    if selection.mode == SelectionMode::TwoStage && !tactical_enabled && two_stage_allowed {
        return two_stage_record(
            &request,
            observation,
            &selection,
            &catalog,
            model,
            gate,
            exchange,
        );
    }
    let options = if tactical_enabled
        && catalog.len() <= tactical::MAX_OPTIONS
        && (selection.presented.is_empty() || selection.presented.len() == catalog.len())
    {
        tactical::catalog_options(observation, &catalog)
    } else {
        present(&selection, observation, &catalog)
    };
    let state = state_with_derived_facts(&request, observation)?;
    let asked = ask::build(&ask::AskContext {
        model,
        state: &state,
        options: &options,
        catalog_count: catalog.len(),
        observation,
        objective: &framing(
            request["objective"].as_str().unwrap_or_default(),
            observation,
        ),
        hard_constraints: &constraints(&request),
        tactical_enabled,
        two_stage_allowed,
    })?;
    // The record states how the ask was shaped. When the split was suppressed the whole set was
    // asked in one question, so the mode is `single` even though the set exceeded the bound.
    let ask_mode = if selection.mode == SelectionMode::TwoStage {
        SelectionMode::Single
    } else {
        selection.mode
    };
    exchange_record(&asked, ask_mode, gate, exchange)
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
        "selection_mode": selection.mode,
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

/// Asks a two-stage question, one `kind` stage then one `action` stage, and records both.
///
/// The action stage keeps the `provider_request`, `provider_response`, and `decision` fields, so a
/// published evidence file reads the same decision-from-response pair it did before. The kind stage
/// is carried beside it under `class_question`, and `selection_mode` states that two stages were
/// used, so a two-stage ask is observable in a record.
fn two_stage_record(
    request: &Value,
    observation: &Value,
    selection: &OptionSelection,
    catalog: &[String],
    model: &str,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    let state = state_with_derived_facts(request, observation)?;
    let objective = framing(
        request["objective"].as_str().unwrap_or_default(),
        observation,
    );
    let constraints = constraints(request);

    let classes = selection.classes();
    let class_body =
        build_class_system_one_request(model, &state, &classes, &objective, &constraints)?;
    let class_response = exchange(&serde_json::to_vec(&class_body)?)?;
    if class_response.len() > LIMIT {
        return Err("provider response exceeds bound".into());
    }
    let class_response: Value = serde_json::from_slice(&class_response)?;
    let chosen_kind = decision::chosen_option(&class_response, KIND_QUESTION, &classes)?;

    let wanted: Vec<&str> = selection
        .options_of_kind(&chosen_kind)
        .iter()
        .map(|option| option.action_id.as_str())
        .collect();
    let options: Vec<_> = present(selection, observation, catalog)
        .into_iter()
        .filter(|option| wanted.contains(&option.id.as_str()))
        .collect();
    if options.is_empty() {
        return Err("the chosen kind has no presented options".into());
    }
    let ids: Vec<String> = options.iter().map(|option| option.id.clone()).collect();
    let action_body =
        build_described_system_one_request(model, &state, &options, &objective, &constraints)?;
    let action_response = exchange(&serde_json::to_vec(&action_body)?)?;
    if action_response.len() > LIMIT {
        return Err("provider response exceeds bound".into());
    }
    let action_response: Value = serde_json::from_slice(&action_response)?;
    let decision = decision::map_decision(&action_response, ACTION_QUESTION, &ids, gate)?;

    Ok(json!({
        "schema": RECORD_SCHEMA,
        "provider_call": true,
        "selection_mode": SelectionMode::TwoStage,
        "class_question": {
            "provider_request": class_body,
            "provider_response": class_response,
        },
        "provider_request": action_body,
        "provider_response": action_response,
        "decision": decision,
    }))
}

fn exchange_record(
    asked: &ask::Ask,
    mode: SelectionMode,
    gate: f64,
    exchange: &mut Exchange<'_>,
) -> Result<Value, Failure> {
    let mut body = asked.body.clone();
    let prepared = if asked.tactical_enabled {
        Some(tactical::prepare(body.clone(), asked.catalog_count)?)
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
    let mut record = finish_record(
        body,
        serde_json::from_slice(&response)?,
        prepared,
        &asked.ids,
        asked.card_choice.as_ref(),
        gate,
    )?;
    record["selection_mode"] = json!(mode);
    Ok(record)
}

fn finish_record(
    body: Value,
    response: Value,
    prepared: Option<tactical::Prepared>,
    ids: &[String],
    card_choice: Option<&DisclosedCardChoice>,
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
    // The card answer is recorded beside the action it qualifies, and it stays advisory. A card
    // identity disclosed on a reward screen belongs to the *next* screen's catalog, so it is not
    // in this state's legal-action set and dispatching it would be an action the host never
    // offered. Reading it here is what makes the question consumed rather than a spend that buys a
    // number no code looks at.
    //
    // Once the question has been asked, its answer is *required*. A question the bridge spent a
    // provider call on, and then discarded, would be the unconsumed question the request contract
    // warns against, and would buy the worst outcome of all: a run that believes it ranked the
    // cards and records no ranking. So a missing, mistyped, choice-less or out-of-set card answer
    // fails the exchange rather than recording `null` and letting the action decision stand. The
    // refusal is the same containment check the action answer gets, applied to the answer this
    // bridge itself asked for.
    if let Some(choice) = card_choice {
        let answered = choice
            .options
            .iter()
            .map(|option| option.id.clone())
            .collect::<Vec<_>>();
        let advisory = decision::chosen_option(&response, CARD_CHOICE_QUESTION, &answered)?;
        record["card_choice"] = json!({
            "reward_id": choice.reward_id,
            "advisory": advisory,
        });
    }
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
