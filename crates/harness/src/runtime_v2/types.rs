// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RuntimeV2CombatPhase {
    #[serde(rename = "outside_combat")]
    OutsideCombat,
    #[serde(rename = "combat/player_turn")]
    PlayerTurn,
    #[serde(rename = "combat/enemy_turn")]
    EnemyTurn,
}

impl RuntimeV2CombatPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OutsideCombat => "outside_combat",
            Self::PlayerTurn => "combat/player_turn",
            Self::EnemyTurn => "combat/enemy_turn",
        }
    }
}

/// The only action in the frozen Runtime-v2 gameplay profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Action {
    action_id: String,
}

impl RuntimeV2Action {
    /// Constructs the argument-free `end_turn` action.
    #[must_use]
    pub fn end_turn() -> Self {
        Self {
            action_id: ACTION_ID.to_owned(),
        }
    }

    /// Parses an action while enforcing the frozen action identity.
    pub fn new(action_id: impl Into<String>) -> Result<Self, RuntimeV2Error> {
        let action_id = action_id.into();
        if action_id != ACTION_ID {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 only permits the end_turn action",
            ));
        }
        Ok(Self { action_id })
    }

    /// Returns the action identity. Runtime-v2 actions have no arguments.
    #[must_use]
    pub fn action_id(&self) -> &str {
        &self.action_id
    }

    fn validate(&self) -> Result<(), RuntimeV2Error> {
        if self.action_id != ACTION_ID {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 action identity is not end_turn",
            ));
        }
        Ok(())
    }
}

/// The bounded observation carried by Runtime-v2 state and settled results.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Observation {
    pub combat_phase: RuntimeV2CombatPhase,
    pub turn_index: u64,
    pub host_ready: bool,
    pub generation: u64,
}

impl RuntimeV2Observation {
    /// Constructs an observation after checking the contract bounds.
    pub fn new(
        combat_phase: RuntimeV2CombatPhase,
        turn_index: u64,
        host_ready: bool,
        generation: u64,
    ) -> Result<Self, RuntimeV2Error> {
        if turn_index > RUNTIME_V2_MAX_TURN_INDEX {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 turn_index exceeds its bound",
            ));
        }
        if generation > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 observation generation exceeds its bound",
            ));
        }
        Ok(Self {
            combat_phase,
            turn_index,
            host_ready,
            generation,
        })
    }

    fn validate(&self) -> Result<(), RuntimeV2Error> {
        Self::new(
            self.combat_phase,
            self.turn_index,
            self.host_ready,
            self.generation,
        )
        .map(|_| ())
    }
}

/// The Runtime-v2 effect witness required for a settled action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2EffectWitness {
    pub kind: String,
    pub generation: u64,
}

impl RuntimeV2EffectWitness {
    /// Constructs the only settled effect witness in the frozen profile.
    pub fn turn_end_settled(generation: u64) -> Result<Self, RuntimeV2Error> {
        if generation > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 witness generation exceeds its bound",
            ));
        }
        Ok(Self {
            kind: WITNESS_KIND.to_owned(),
            generation,
        })
    }

    fn validate(&self) -> Result<(), RuntimeV2Error> {
        if self.kind != WITNESS_KIND || self.generation > RUNTIME_V2_MAX_GENERATION {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 effect witness is invalid",
            ));
        }
        Ok(())
    }
}

/// The six wire message kinds in Runtime-v2.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2Kind {
    StateRequest,
    StateResponse,
    ActionRequest,
    ActionResponse,
    ReconcileRequest,
    ReconcileResponse,
}

impl RuntimeV2Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StateRequest => "state_request",
            Self::StateResponse => "state_response",
            Self::ActionRequest => "action_request",
            Self::ActionResponse => "action_response",
            Self::ReconcileRequest => "reconcile_request",
            Self::ReconcileResponse => "reconcile_response",
        }
    }
}

/// The five operation outcomes allowed by the Runtime-v2 profile.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeV2Status {
    Accepted,
    Settled,
    Rejected,
    Unknown,
    Cancelled,
}

impl RuntimeV2Status {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Settled => "settled",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
            Self::Cancelled => "cancelled",
        }
    }
}

/// A bounded operation identity allocated before an action is submitted.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RuntimeV2OperationId(String);

impl RuntimeV2OperationId {
    /// Creates an operation identity after checking the shared identity bound.
    pub fn new(value: impl Into<String>) -> Result<Self, RuntimeV2Error> {
        let value = value.into();
        validate_identity(&value)?;
        Ok(Self(value))
    }

    /// Returns the stable operation identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The instance/session/lease fence carried by every Runtime-v2 message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeV2Context {
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
}

impl RuntimeV2Context {
    /// Constructs a fenced runtime context.
    pub fn new(
        instance_id: impl Into<String>,
        session_id: impl Into<String>,
        lease_id: impl Into<String>,
        lease_epoch: u64,
    ) -> Result<Self, RuntimeV2Error> {
        let context = Self {
            instance_id: instance_id.into(),
            session_id: session_id.into(),
            lease_id: lease_id.into(),
            lease_epoch,
        };
        context.validate()?;
        Ok(context)
    }

    /// Returns the instance identity.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Returns the session identity.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Returns the lease identity.
    #[must_use]
    pub fn lease_id(&self) -> &str {
        &self.lease_id
    }

    /// Returns the lease epoch.
    #[must_use]
    pub const fn lease_epoch(&self) -> u64 {
        self.lease_epoch
    }

    fn validate(&self) -> Result<(), RuntimeV2Error> {
        validate_identity(&self.instance_id)?;
        validate_identity(&self.session_id)?;
        validate_identity(&self.lease_id)?;
        if self.lease_epoch > RUNTIME_V2_MAX_LEASE_EPOCH {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 lease_epoch exceeds its bound",
            ));
        }
        Ok(())
    }
}

/// The provenance attached to every Runtime-v2 envelope and artifact record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Provenance {
    pub artifact: String,
    pub source: String,
    pub generator: String,
}

impl Default for RuntimeV2Provenance {
    fn default() -> Self {
        Self {
            artifact: RUNTIME_V2_ARTIFACT.to_owned(),
            source: RUNTIME_V2_SCHEMA_SOURCE.to_owned(),
            generator: RUNTIME_V2_GENERATOR.to_owned(),
        }
    }
}

impl RuntimeV2Provenance {
    fn validate(&self) -> Result<(), RuntimeV2Error> {
        if self.artifact != RUNTIME_V2_ARTIFACT
            || self.source != RUNTIME_V2_SCHEMA_SOURCE
            || self.generator != RUNTIME_V2_GENERATOR
        {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 provenance does not identify the copied artifact",
            ));
        }
        Ok(())
    }
}
