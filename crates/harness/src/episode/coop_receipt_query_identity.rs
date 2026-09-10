// SPDX-License-Identifier: MIT

use serde_json::{Map, Value};

#[path = "coop_receipt_query_identity_validation.rs"]
mod validation;

const FIELDS: [&str; 12] = [
    "operation_id",
    "action_kind",
    "action_fingerprint",
    "session_id",
    "run_id",
    "location",
    "actor_id",
    "authority_id",
    "authority_epoch",
    "expected_host_generation",
    "before_host_generation",
    "participant_ids",
];

/// One coordinate in the location captured when the operation was admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiptQueryCoordinate {
    col: i32,
    row: i32,
}

impl ReceiptQueryCoordinate {
    #[must_use]
    pub const fn new(col: i32, row: i32) -> Self {
        Self { col, row }
    }

    #[must_use]
    pub const fn col(self) -> i32 {
        self.col
    }

    #[must_use]
    pub const fn row(self) -> i32 {
        self.row
    }
}

/// Immutable map location associated with an admitted co-op operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReceiptQueryLocation {
    act_index: i32,
    room_id: Option<i32>,
    coordinate: Option<ReceiptQueryCoordinate>,
}

impl ReceiptQueryLocation {
    #[must_use]
    pub const fn new(
        act_index: i32,
        room_id: Option<i32>,
        coordinate: Option<ReceiptQueryCoordinate>,
    ) -> Self {
        Self {
            act_index,
            room_id,
            coordinate,
        }
    }

    #[must_use]
    pub const fn act_index(self) -> i32 {
        self.act_index
    }

    #[must_use]
    pub const fn room_id(self) -> Option<i32> {
        self.room_id
    }

    #[must_use]
    pub const fn coordinate(self) -> Option<ReceiptQueryCoordinate> {
        self.coordinate
    }
}

/// Action families supported by the neutral retained receipt profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptQueryActionKind {
    EndTurn,
    PlayCard,
}

impl ReceiptQueryActionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::PlayCard => "play_card",
        }
    }

    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "end_turn" => Some(Self::EndTurn),
            "play_card" => Some(Self::PlayCard),
            _ => None,
        }
    }
}

/// Full immutable identity of one originally admitted co-op operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReceiptQueryIdentity {
    operation_id: String,
    action_kind: ReceiptQueryActionKind,
    action_fingerprint: String,
    session_id: String,
    run_id: String,
    location: ReceiptQueryLocation,
    actor_id: String,
    authority_id: String,
    authority_epoch: String,
    expected_host_generation: u64,
    before_host_generation: u64,
    participant_ids: Vec<String>,
}

impl ReceiptQueryIdentity {
    /// Constructs and validates an immutable operation identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: impl Into<String>,
        action_kind: ReceiptQueryActionKind,
        action_fingerprint: impl Into<String>,
        session_id: impl Into<String>,
        run_id: impl Into<String>,
        location: ReceiptQueryLocation,
        actor_id: impl Into<String>,
        authority_id: impl Into<String>,
        authority_epoch: impl Into<String>,
        expected_host_generation: u64,
        before_host_generation: u64,
        participant_ids: Vec<String>,
    ) -> Result<Self, ReceiptQueryIdentityError> {
        let identity = Self {
            operation_id: operation_id.into(),
            action_kind,
            action_fingerprint: action_fingerprint.into(),
            session_id: session_id.into(),
            run_id: run_id.into(),
            location,
            actor_id: actor_id.into(),
            authority_id: authority_id.into(),
            authority_epoch: authority_epoch.into(),
            expected_host_generation,
            before_host_generation,
            participant_ids,
        };
        identity.validate()?;
        Ok(identity)
    }

    pub(super) fn from_object(
        object: &Map<String, Value>,
    ) -> Result<Self, ReceiptQueryIdentityError> {
        if FIELDS.iter().any(|field| !object.contains_key(*field)) {
            return Err(ReceiptQueryIdentityError::InvalidShape);
        }
        let action_kind = object
            .get("action_kind")
            .and_then(Value::as_str)
            .and_then(ReceiptQueryActionKind::parse)
            .ok_or(ReceiptQueryIdentityError::InvalidActionKind)?;
        let participants = object
            .get("participant_ids")
            .and_then(Value::as_array)
            .ok_or(ReceiptQueryIdentityError::InvalidParticipants)?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .filter(|value| validation::safe(value))
                    .map(str::to_owned)
                    .ok_or(ReceiptQueryIdentityError::InvalidParticipants)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(
            validation::text(object, "operation_id")?,
            action_kind,
            validation::digest(object, "action_fingerprint")?,
            validation::text(object, "session_id")?,
            validation::text(object, "run_id")?,
            validation::parse_location(object.get("location"))?,
            validation::text(object, "actor_id")?,
            validation::text(object, "authority_id")?,
            validation::text(object, "authority_epoch")?,
            validation::generation(object.get("expected_host_generation"))?,
            validation::generation(object.get("before_host_generation"))?,
            participants,
        )
    }

    fn validate(&self) -> Result<(), ReceiptQueryIdentityError> {
        if [
            self.operation_id.as_str(),
            self.session_id.as_str(),
            self.run_id.as_str(),
            self.actor_id.as_str(),
            self.authority_id.as_str(),
            self.authority_epoch.as_str(),
        ]
        .iter()
        .any(|value| !validation::safe(value))
        {
            return Err(ReceiptQueryIdentityError::InvalidIdentity);
        }
        if !validation::hex(&self.action_fingerprint) {
            return Err(ReceiptQueryIdentityError::InvalidDigest);
        }
        if self.expected_host_generation != self.before_host_generation
            || self.before_host_generation > validation::MAX_GENERATION
        {
            return Err(ReceiptQueryIdentityError::InvalidGeneration);
        }
        if !(2..=4).contains(&self.participant_ids.len())
            || self
                .participant_ids
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || self
                .participant_ids
                .iter()
                .any(|value| !validation::safe(value))
            || !self
                .participant_ids
                .iter()
                .any(|value| value == &self.actor_id)
        {
            return Err(ReceiptQueryIdentityError::InvalidParticipants);
        }
        Ok(())
    }

    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    #[must_use]
    pub const fn action_kind(&self) -> ReceiptQueryActionKind {
        self.action_kind
    }

    #[must_use]
    pub fn action_fingerprint(&self) -> &str {
        &self.action_fingerprint
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    #[must_use]
    pub const fn location(&self) -> ReceiptQueryLocation {
        self.location
    }

    #[must_use]
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    #[must_use]
    pub fn authority_id(&self) -> &str {
        &self.authority_id
    }

    #[must_use]
    pub fn authority_epoch(&self) -> &str {
        &self.authority_epoch
    }

    #[must_use]
    pub const fn expected_host_generation(&self) -> u64 {
        self.expected_host_generation
    }

    #[must_use]
    pub const fn before_host_generation(&self) -> u64 {
        self.before_host_generation
    }

    #[must_use]
    pub fn participant_ids(&self) -> &[String] {
        &self.participant_ids
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReceiptQueryIdentityError {
    InvalidShape,
    InvalidIdentity,
    InvalidActionKind,
    InvalidDigest,
    InvalidGeneration,
    InvalidParticipants,
    InvalidLocation,
}

impl std::fmt::Display for ReceiptQueryIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidShape => "receipt-query identity shape is invalid",
            Self::InvalidIdentity => "receipt-query identity is invalid",
            Self::InvalidActionKind => "receipt-query action kind is invalid",
            Self::InvalidDigest => "receipt-query action fingerprint is invalid",
            Self::InvalidGeneration => "receipt-query generation is invalid",
            Self::InvalidParticipants => "receipt-query participants are invalid",
            Self::InvalidLocation => "receipt-query location is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ReceiptQueryIdentityError {}

#[cfg(test)]
#[path = "coop_receipt_query_identity_tests.rs"]
mod tests;
