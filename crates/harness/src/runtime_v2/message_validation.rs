// SPDX-License-Identifier: MIT

impl RuntimeV2Message {

    pub fn from_json(value: &str) -> Result<Self, RuntimeV2Error> {
        let message: Self = serde_json::from_str(value).map_err(|_| RuntimeV2Error::Decode)?;
        message.validate()?;
        Ok(message)
    }

    /// Returns canonical JSON bytes for this validated message.
    pub fn to_json(&self) -> Result<String, RuntimeV2Error> {
        self.validate()?;
        let encoded = serde_json::to_string(self).map_err(|_| RuntimeV2Error::Encode)?;
        let decoded = Self::from_json(&encoded)?;
        if decoded != *self {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 message changed during JSON round-trip",
            ));
        }
        Ok(encoded)
    }

    /// Validates the full envelope and conditional message shape.
    pub fn validate(&self) -> Result<(), RuntimeV2Error> {
        if self.protocol_version != RUNTIME_V2_PROTOCOL_VERSION
            || self.schema_digest != RUNTIME_V2_SCHEMA_DIGEST
        {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 version or schema digest is invalid",
            ));
        }
        self.provenance.validate()?;
        validate_identity(&self.correlation_id)?;
        validate_identity(&self.instance_id)?;
        validate_identity(&self.session_id)?;
        validate_identity(&self.lease_id)?;
        if self.lease_epoch > RUNTIME_V2_MAX_LEASE_EPOCH
            || self.generation > RUNTIME_V2_MAX_GENERATION
        {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 lease epoch or generation exceeds its bound",
            ));
        }
        if let Some(operation_id) = &self.operation_id {
            validate_identity(operation_id.as_str())?;
        }
        if let Some(observation) = self.observation {
            observation.validate()?;
            if observation.generation != self.generation {
                return Err(RuntimeV2Error::Invalid(
                    "Runtime-v2 observation generation does not match its envelope",
                ));
            }
        }
        if let Some(action) = &self.action {
            action.validate()?;
        }
        if let Some(error_code) = &self.error_code {
            validate_identity(error_code)?;
        }
        if let Some(witness) = &self.effect_witness {
            witness.validate()?;
            if witness.generation != self.generation {
                return Err(RuntimeV2Error::Invalid(
                    "Runtime-v2 witness generation does not match its envelope",
                ));
            }
        }

        match self.kind {
            RuntimeV2Kind::StateRequest => self.validate_state_request(),
            RuntimeV2Kind::StateResponse => self.validate_state_response(),
            RuntimeV2Kind::ActionRequest => self.validate_action_request(),
            RuntimeV2Kind::ActionResponse => self.validate_result(false),
            RuntimeV2Kind::ReconcileRequest => self.validate_reconcile_request(),
            RuntimeV2Kind::ReconcileResponse => self.validate_result(true),
        }
    }

    /// Returns the message kind.
    #[must_use]
    pub const fn kind(&self) -> RuntimeV2Kind {
        self.kind
    }

    /// Returns the correlation identity.
    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
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

    /// Returns the envelope generation.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the operation identity, when this is an action/reconcile message.
    #[must_use]
    pub fn operation_id(&self) -> Option<&RuntimeV2OperationId> {
        self.operation_id.as_ref()
    }

    /// Returns the observation, when the message carries one.
    #[must_use]
    pub const fn observation(&self) -> Option<RuntimeV2Observation> {
        self.observation
    }

    /// Returns the action, when the message carries one.
    #[must_use]
    pub fn action(&self) -> Option<&RuntimeV2Action> {
        self.action.as_ref()
    }

    /// Returns the outcome status, when the message is a result.
    #[must_use]
    pub const fn status(&self) -> Option<RuntimeV2Status> {
        self.status
    }

    /// Returns the error identity, when the result is rejected/unknown/cancelled.
    #[must_use]
    pub fn error_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    /// Returns the settlement witness, when the result is settled.
    #[must_use]
    pub fn effect_witness(&self) -> Option<&RuntimeV2EffectWitness> {
        self.effect_witness.as_ref()
    }

    fn context(&self) -> RuntimeV2Context {
        RuntimeV2Context {
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            lease_id: self.lease_id.clone(),
            lease_epoch: self.lease_epoch,
        }
    }

    fn validate_state_request(&self) -> Result<(), RuntimeV2Error> {
        if self.operation_id.is_none()
            && self.observation.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2Error::Invalid(
                "Runtime-v2 state request has the wrong shape",
            ))
        }
    }

    fn validate_state_response(&self) -> Result<(), RuntimeV2Error> {
        if self.operation_id.is_none()
            && self.observation.is_some()
            && self.action.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2Error::Invalid(
                "Runtime-v2 state response has the wrong shape",
            ))
        }
    }

    fn validate_action_request(&self) -> Result<(), RuntimeV2Error> {
        if self.operation_id.is_some()
            && self.observation.is_none()
            && self.action.is_some()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2Error::Invalid(
                "Runtime-v2 action request has the wrong shape",
            ))
        }
    }

    fn validate_reconcile_request(&self) -> Result<(), RuntimeV2Error> {
        if self.operation_id.is_some()
            && self.observation.is_none()
            && self.action.is_none()
            && self.status.is_none()
            && self.error_code.is_none()
            && self.effect_witness.is_none()
        {
            Ok(())
        } else {
            Err(RuntimeV2Error::Invalid(
                "Runtime-v2 reconcile request has the wrong shape",
            ))
        }
    }

    fn validate_result(&self, reconcile: bool) -> Result<(), RuntimeV2Error> {
        if self.operation_id.is_none() || self.action.is_none() || self.status.is_none() {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 result is missing operation, action, or status",
            ));
        }
        if reconcile && self.kind != RuntimeV2Kind::ReconcileResponse {
            return Err(RuntimeV2Error::Invalid(
                "Runtime-v2 reconciliation result kind is invalid",
            ));
        }
        match self.status {
            Some(RuntimeV2Status::Accepted) => {
                if self.observation.is_some()
                    && self.error_code.is_none()
                    && self.effect_witness.is_none()
                {
                    Ok(())
                } else {
                    Err(RuntimeV2Error::Invalid(
                        "accepted Runtime-v2 result is not admission-only",
                    ))
                }
            }
            Some(RuntimeV2Status::Settled) => {
                if self.observation.is_some()
                    && self.error_code.is_none()
                    && self.effect_witness.is_some()
                {
                    Ok(())
                } else {
                    Err(RuntimeV2Error::Invalid(
                        "settled Runtime-v2 result lacks a fresh witness",
                    ))
                }
            }
            Some(RuntimeV2Status::Rejected) => {
                if self.observation.is_some()
                    && self.error_code.is_some()
                    && self.effect_witness.is_none()
                {
                    Ok(())
                } else {
                    Err(RuntimeV2Error::Invalid(
                        "rejected Runtime-v2 result has the wrong mutation evidence",
                    ))
                }
            }
            Some(RuntimeV2Status::Unknown) => {
                if self.observation.is_none()
                    && self.error_code.is_some()
                    && self.effect_witness.is_none()
                {
                    Ok(())
                } else {
                    Err(RuntimeV2Error::Invalid(
                        "unknown Runtime-v2 result must not claim an observation",
                    ))
                }
            }
            Some(RuntimeV2Status::Cancelled) => {
                if self.observation.is_some()
                    && self.error_code.is_some()
                    && self.effect_witness.is_none()
                {
                    Ok(())
                } else {
                    Err(RuntimeV2Error::Invalid(
                        "cancelled Runtime-v2 result has the wrong shape",
                    ))
                }
            }
            None => Err(RuntimeV2Error::Invalid("Runtime-v2 result has no status")),
        }
    }
}
