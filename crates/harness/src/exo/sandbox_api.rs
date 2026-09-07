// SPDX-License-Identifier: MIT

/// JSON projection that admits only ordinary player-visible fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SanitizedObservation(Value);

impl SanitizedObservation {
    /// Validates a projection before it enters an Exo request.
    pub fn new(value: Value) -> Result<Self, SandboxError> {
        let encoded = serde_json::to_vec(&value).map_err(|_| SandboxError::MalformedJson)?;
        if encoded.len() > MAX_OBSERVATION_BYTES {
            return Err(SandboxError::TooLarge);
        }
        if value.get("protocol_version").and_then(Value::as_str)
            == Some(crate::RUNTIME_V4_EXPERT_PROTOCOL_VERSION)
        {
            crate::RuntimeV4ExpertObservation::from_value(value.clone())
                .map_err(|_| SandboxError::InvalidExpertObservation)?;
            return Ok(Self(value));
        }
        validate_value(&value, ValueKind::Root, true)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    #[must_use]
    pub fn state_id(&self) -> Option<&str> {
        self.0.get("state_id").and_then(Value::as_str)
    }

    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        self.0.get("generation").and_then(Value::as_u64)
    }

    /// Reports whether the host-visible seed text is still part of this projection.
    #[must_use]
    pub fn has_visible_seed(&self) -> bool {
        self.0
            .get("visible_seed")
            .is_some_and(|value| !value.is_null())
    }

    /// Drops `visible_seed` for an explicitly seed-blind experiment.
    #[must_use]
    pub fn without_visible_seed(mut self) -> Self {
        if let Value::Object(object) = &mut self.0 {
            if object.get("protocol_version").and_then(Value::as_str)
                == Some(crate::RUNTIME_V4_EXPERT_PROTOCOL_VERSION)
            {
                object.insert("visible_seed".to_owned(), Value::Null);
            } else {
                object.remove("visible_seed");
            }
        }
        self
    }
}
/// A rejected projection never reaches a provider transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxError {
    MalformedJson,
    TooLarge,
    NotAnObservation,
    UnknownField,
    PrivilegedField,
    InvalidText,
    InvalidNumber,
    InvalidCollection,
    DuplicateLegalAction,
    InvalidExpertObservation,
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::MalformedJson => "fair-play observation could not be encoded",
            Self::TooLarge => "fair-play observation exceeds its byte bound",
            Self::NotAnObservation => {
                "fair-play observation must be an object with state and actions"
            }
            Self::UnknownField => "fair-play observation contains an unknown field",
            Self::PrivilegedField => "fair-play observation contains a privileged field",
            Self::InvalidText => "fair-play observation contains invalid text",
            Self::InvalidNumber => "fair-play observation contains an invalid number",
            Self::InvalidCollection => "fair-play observation contains an oversized collection",
            Self::DuplicateLegalAction => "fair-play observation contains a duplicate legal action",
            Self::InvalidExpertObservation => {
                "fair-play observation contains an invalid Runtime-v4 expert state"
            }
        })
    }
}

impl std::error::Error for SandboxError {}
