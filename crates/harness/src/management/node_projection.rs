// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::episode::{EpisodeLegalActionSet, EpisodeObservation};
use crate::workflow::{
    BoundedText, Digest, Generation, ObservationValue, RuntimeFault, ScalarValue,
};

pub(super) fn observation_value(
    observation: &EpisodeObservation,
) -> Result<ObservationValue, RuntimeFault> {
    let state_id =
        BoundedText::new(observation.state_id()).map_err(|_| RuntimeFault::InvalidState)?;
    let generation =
        Generation::new(observation.generation()).map_err(|_| RuntimeFault::InvalidState)?;
    let fields = observation
        .fair_play()
        .as_value()
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| scalar_value(value).map(|item| (key.clone(), item)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    Ok(ObservationValue {
        state_id,
        generation,
        fields,
    })
}

fn scalar_value(value: &Value) -> Option<ScalarValue> {
    match value {
        Value::Null => Some(ScalarValue::Null),
        Value::Bool(item) => Some(ScalarValue::Boolean(*item)),
        Value::String(item) => BoundedText::new(item).ok().map(ScalarValue::Text),
        Value::Number(item) => item.as_i64().map(ScalarValue::Integer).or_else(|| {
            item.as_u64()
                .and_then(|value| i64::try_from(value).ok())
                .map(ScalarValue::Integer)
        }),
        Value::Array(_) | Value::Object(_) => None,
    }
}

pub(super) fn catalog_digest(actions: &EpisodeLegalActionSet) -> Result<Digest, RuntimeFault> {
    let values = actions
        .actions()
        .iter()
        .map(|action| {
            json!({"action_id": action.action_id(), "kind": format!("{:?}", action.kind())})
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&values).map_err(|_| RuntimeFault::InvalidState)?;
    Ok(Digest::sha256(&bytes))
}
