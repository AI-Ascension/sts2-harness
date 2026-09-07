// SPDX-License-Identifier: MIT

/// A parsed, host-produced Runtime-v4 expert observation.
///
/// The raw value is retained because the existing Exo request contract is JSON based. The typed
/// wire value is private and is validated before the value can cross the provider boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV4ExpertObservation {
    value: Value,
    wire: WireObservation,
}

impl RuntimeV4ExpertObservation {
    /// Parses and validates one serialized gateway/MCP response.
    pub fn parse(bytes: &[u8]) -> Result<Self, RuntimeV4ExpertParseError> {
        if bytes.len() > MAX_OBSERVATION_BYTES {
            return Err(RuntimeV4ExpertParseError::TooLarge);
        }
        let mut deserializer = serde_json::Deserializer::from_slice(bytes);
        let StrictJsonValue(value) = StrictJsonValue::deserialize(&mut deserializer)
            .map_err(|_| RuntimeV4ExpertParseError::MalformedJson)?;
        deserializer
            .end()
            .map_err(|_| RuntimeV4ExpertParseError::MalformedJson)?;
        Self::from_value(value)
    }

    /// Validates an already decoded gateway/MCP response.
    pub fn from_value(value: Value) -> Result<Self, RuntimeV4ExpertParseError> {
        let encoded =
            serde_json::to_vec(&value).map_err(|_| RuntimeV4ExpertParseError::MalformedJson)?;
        if encoded.len() > MAX_OBSERVATION_BYTES {
            return Err(RuntimeV4ExpertParseError::TooLarge);
        }
        if !shape_is_closed(&value) {
            return Err(RuntimeV4ExpertParseError::InvalidShape);
        }
        let wire: WireObservation = serde_json::from_value(value.clone())
            .map_err(|_| RuntimeV4ExpertParseError::InvalidShape)?;
        validate_wire(&wire)?;
        verify_runtime_v4_expert_artifact().map_err(RuntimeV4ExpertParseError::ArtifactMismatch)?;
        Ok(Self { value, wire })
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub fn state_id(&self) -> &str {
        &self.wire.state_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.wire.generation
    }

    pub fn legal_action_ids(&self) -> impl Iterator<Item = &str> {
        self.wire
            .legal_actions
            .iter()
            .map(|action| action.action_id.as_str())
    }

    /// Returns the provider-facing observation after the same fair-play validation used by Exo.
    /// This method is useful to callers that already have a decoded v4 response and want to
    /// construct the ordinary provider request prompt.
    pub fn into_sanitized(self) -> Result<crate::SanitizedObservation, crate::SandboxError> {
        crate::SanitizedObservation::new(self.value)
    }
}

/// `serde_json::Value` keeps the last value for a duplicate object key. The host response is a
/// trust boundary, so the byte parser uses this small recursive value visitor to reject duplicate
/// keys before the ordinary closed-shape validation sees the object.
struct StrictJsonValue(Value);

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrictVisitor;

        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictJsonValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value with unique object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::from(value)))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::from(value)))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::from(value)))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::String(value.to_owned())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::String(value)))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::Null))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StrictJsonValue(Value::Null))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                StrictJsonValue::deserialize(deserializer)
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictJsonValue>()? {
                    values.push(value.0);
                }
                Ok(StrictJsonValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut object = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(de::Error::custom("duplicate JSON object key"));
                    }
                    let value = map.next_value::<StrictJsonValue>()?;
                    object.insert(key, value.0);
                }
                Ok(StrictJsonValue(Value::Object(object)))
            }
        }

        deserializer.deserialize_any(StrictVisitor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV4ExpertParseError {
    TooLarge,
    MalformedJson,
    InvalidShape,
    InvalidValue,
    ArtifactMismatch(crate::RuntimeV4ExpertArtifactError),
}

impl std::fmt::Display for RuntimeV4ExpertParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::TooLarge => "runtime-v4-expert observation exceeds its byte bound",
            Self::MalformedJson => "runtime-v4-expert observation is malformed JSON",
            Self::InvalidShape => "runtime-v4-expert observation has an invalid closed shape",
            Self::InvalidValue => "runtime-v4-expert observation has an invalid value",
            Self::ArtifactMismatch(_) => "runtime-v4-expert artifact verification failed",
        })
    }
}

impl std::error::Error for RuntimeV4ExpertParseError {}
