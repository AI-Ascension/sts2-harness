// SPDX-License-Identifier: MIT

const MAX_DECISION_BYTES: usize = 8 * 1024;
const MAX_RATIONALE_BYTES: usize = 512;

/// The only semantic choices a model may return.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Action {
        action_id: String,
        rationale: String,
        confidence: Option<u8>,
    },
    Wait {
        rationale: String,
    },
    Reobserve {
        rationale: String,
    },
    Recovery {
        kind: String,
        operation_id: Option<String>,
        rationale: String,
    },
}

/// A model action bound to the host-generated current catalog.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundDecision {
    pub action_id: String,
    pub rationale: String,
    pub confidence: Option<u8>,
}

/// Strict parser errors. Extra fields and chain-of-thought-shaped fields are rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionError {
    TooLarge,
    InvalidJson,
    UnknownField,
    MissingField,
    InvalidValue,
    IllegalAction,
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "model decision exceeds its byte bound",
            Self::InvalidJson => "model decision is not valid JSON",
            Self::UnknownField => "model decision contains an unknown field",
            Self::MissingField => "model decision is missing a required field",
            Self::InvalidValue => "model decision contains an invalid value",
            Self::IllegalAction => "model action is not in the current host catalog",
        })
    }
}

impl std::error::Error for DecisionError {}

/// Parses one bounded structured response without retaining a verbatim model transcript.
pub fn parse_decision(bytes: &[u8]) -> Result<Decision, DecisionError> {
    if bytes.len() > MAX_DECISION_BYTES {
        return Err(DecisionError::TooLarge);
    }
    let object = parse_object(bytes)?;
    let allowed = [
        "decision",
        "action_id",
        "rationale",
        "confidence",
        "recovery_kind",
        "operation_id",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(DecisionError::UnknownField);
    }
    let decision = object
        .get("decision")
        .and_then(serde_json::Value::as_str)
        .ok_or(DecisionError::MissingField)?;
    let rationale = object
        .get("rationale")
        .and_then(serde_json::Value::as_str)
        .ok_or(DecisionError::MissingField)?
        .to_owned();
    if rationale.is_empty()
        || rationale.len() > MAX_RATIONALE_BYTES
        || rationale.chars().any(char::is_control)
    {
        return Err(DecisionError::InvalidValue);
    }
    match decision {
        "action" => parse_action(&object, rationale),
        "wait" | "reobserve" => parse_observation_directive(&object, decision, rationale),
        "recovery" => parse_recovery(&object, rationale),
        _ => Err(DecisionError::InvalidValue),
    }
}

fn parse_action(
    object: &serde_json::Map<String, serde_json::Value>,
    rationale: String,
) -> Result<Decision, DecisionError> {
    if object.contains_key("recovery_kind") || object.contains_key("operation_id") {
        return Err(DecisionError::UnknownField);
    }
    let action_id = object
        .get("action_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(DecisionError::MissingField)?;
    if !valid_id(action_id) {
        return Err(DecisionError::InvalidValue);
    }
    let confidence = match object.get("confidence") {
        None => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .filter(|value| *value <= 100)
                .ok_or(DecisionError::InvalidValue)?,
        ),
    };
    Ok(Decision::Action {
        action_id: action_id.to_owned(),
        rationale,
        confidence,
    })
}

fn parse_recovery(
    object: &serde_json::Map<String, serde_json::Value>,
    rationale: String,
) -> Result<Decision, DecisionError> {
    if object.contains_key("action_id") || object.contains_key("confidence") {
        return Err(DecisionError::UnknownField);
    }
    let kind = object
        .get("recovery_kind")
        .and_then(serde_json::Value::as_str)
        .ok_or(DecisionError::MissingField)?;
    if !matches!(
        kind,
        "reobserve" | "reconcile" | "release_lease" | "stop_episode"
    ) {
        return Err(DecisionError::InvalidValue);
    }
    let operation_id = object
        .get("operation_id")
        .map(|value| {
            value
                .as_str()
                .filter(|value| valid_id(value))
                .map(str::to_owned)
                .ok_or(DecisionError::InvalidValue)
        })
        .transpose()?;
    if (kind == "reconcile") != operation_id.is_some() {
        return Err(DecisionError::MissingField);
    }
    Ok(Decision::Recovery {
        kind: kind.to_owned(),
        operation_id,
        rationale,
    })
}

fn parse_observation_directive(
    object: &serde_json::Map<String, serde_json::Value>,
    decision: &str,
    rationale: String,
) -> Result<Decision, DecisionError> {
    if object.keys().any(|key| {
        matches!(
            key.as_str(),
            "action_id" | "confidence" | "recovery_kind" | "operation_id"
        )
    }) {
        return Err(DecisionError::UnknownField);
    }
    match decision {
        "wait" => Ok(Decision::Wait { rationale }),
        "reobserve" => Ok(Decision::Reobserve { rationale }),
        _ => Err(DecisionError::InvalidValue),
    }
}

fn parse_object(bytes: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>, DecisionError> {
    struct UniqueFields;

    impl<'de> serde::de::Visitor<'de> for UniqueFields {
        type Value = serde_json::Map<String, serde_json::Value>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a decision object with unique field names")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut object = serde_json::Map::new();
            while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                if object.insert(key, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate decision field"));
                }
            }
            Ok(object)
        }
    }

    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let object = serde::Deserializer::deserialize_map(&mut decoder, UniqueFields)
        .map_err(|_| DecisionError::InvalidJson)?;
    decoder.end().map_err(|_| DecisionError::InvalidJson)?;
    Ok(object)
}

impl Decision {
    /// Binds an action decision to the exact current host-generated action IDs.
    pub fn bind(self, legal_action_ids: &[String]) -> Result<BoundDecision, DecisionError> {
        let Decision::Action {
            action_id,
            rationale,
            confidence,
        } = self
        else {
            return Err(DecisionError::InvalidValue);
        };
        if !legal_action_ids
            .iter()
            .any(|candidate| candidate == &action_id)
        {
            return Err(DecisionError::IllegalAction);
        }
        Ok(BoundDecision {
            action_id,
            rationale,
            confidence,
        })
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}
