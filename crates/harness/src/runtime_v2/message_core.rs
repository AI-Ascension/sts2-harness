// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeV2Message {
    protocol_version: String,
    schema_digest: String,
    provenance: RuntimeV2Provenance,
    correlation_id: String,
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    generation: u64,
    kind: RuntimeV2Kind,
    #[serde(deserialize_with = "required_nullable")]
    operation_id: Option<RuntimeV2OperationId>,
    #[serde(deserialize_with = "required_nullable")]
    observation: Option<RuntimeV2Observation>,
    #[serde(deserialize_with = "required_nullable")]
    action: Option<RuntimeV2Action>,
    #[serde(deserialize_with = "required_nullable")]
    status: Option<RuntimeV2Status>,
    #[serde(deserialize_with = "required_nullable")]
    error_code: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    effect_witness: Option<RuntimeV2EffectWitness>,
}

// The frozen schema requires these keys even when their values must be null. A custom
// deserializer disables serde's implicit default for an absent Option field.
fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

impl RuntimeV2Message {
    /// Builds a state request.
    pub fn state_request(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        generation: u64,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            generation,
            RuntimeV2Kind::StateRequest,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// Builds a state response.
    pub fn state_response(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        observation: RuntimeV2Observation,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            observation.generation,
            RuntimeV2Kind::StateResponse,
            None,
            Some(observation),
            None,
            None,
            None,
            None,
        )
    }

    /// Builds an action request carrying its preallocated operation identity.
    pub fn action_request(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        generation: u64,
        operation_id: RuntimeV2OperationId,
        action: RuntimeV2Action,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            generation,
            RuntimeV2Kind::ActionRequest,
            Some(operation_id),
            None,
            Some(action),
            None,
            None,
            None,
        )
    }

    /// Builds an action response for an operation outcome.
    #[allow(clippy::too_many_arguments)]
    pub fn action_response(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        generation: u64,
        operation_id: RuntimeV2OperationId,
        action: RuntimeV2Action,
        observation: Option<RuntimeV2Observation>,
        status: RuntimeV2Status,
        error_code: Option<String>,
        effect_witness: Option<RuntimeV2EffectWitness>,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            generation,
            RuntimeV2Kind::ActionResponse,
            Some(operation_id),
            observation,
            Some(action),
            Some(status),
            error_code,
            effect_witness,
        )
    }

    /// Builds a fixed reconciliation request. It carries only the operation identity.
    pub fn reconcile_request(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        generation: u64,
        operation_id: RuntimeV2OperationId,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            generation,
            RuntimeV2Kind::ReconcileRequest,
            Some(operation_id),
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// Builds a reconciliation response for an operation outcome.
    #[allow(clippy::too_many_arguments)]
    pub fn reconcile_response(
        context: &RuntimeV2Context,
        correlation_id: impl Into<String>,
        generation: u64,
        operation_id: RuntimeV2OperationId,
        action: RuntimeV2Action,
        observation: Option<RuntimeV2Observation>,
        status: RuntimeV2Status,
        error_code: Option<String>,
        effect_witness: Option<RuntimeV2EffectWitness>,
    ) -> Result<Self, RuntimeV2Error> {
        Self::new(
            context,
            correlation_id.into(),
            generation,
            RuntimeV2Kind::ReconcileResponse,
            Some(operation_id),
            observation,
            Some(action),
            Some(status),
            error_code,
            effect_witness,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        context: &RuntimeV2Context,
        correlation_id: String,
        generation: u64,
        kind: RuntimeV2Kind,
        operation_id: Option<RuntimeV2OperationId>,
        observation: Option<RuntimeV2Observation>,
        action: Option<RuntimeV2Action>,
        status: Option<RuntimeV2Status>,
        error_code: Option<String>,
        effect_witness: Option<RuntimeV2EffectWitness>,
    ) -> Result<Self, RuntimeV2Error> {
        let message = Self {
            protocol_version: RUNTIME_V2_PROTOCOL_VERSION.to_owned(),
            schema_digest: RUNTIME_V2_SCHEMA_DIGEST.to_owned(),
            provenance: RuntimeV2Provenance::default(),
            correlation_id,
            instance_id: context.instance_id.clone(),
            session_id: context.session_id.clone(),
            lease_id: context.lease_id.clone(),
            lease_epoch: context.lease_epoch,
            generation,
            kind,
            operation_id,
            observation,
            action,
            status,
            error_code,
            effect_witness,
        };
        message.validate()?;
        Ok(message)
    }


}
