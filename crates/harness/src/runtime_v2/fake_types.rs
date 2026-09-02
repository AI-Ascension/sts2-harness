// SPDX-License-Identifier: MIT

#[derive(Serialize)]
struct RuntimeV2TraceDocument<'a> {
    artifact: &'a RuntimeV2ArtifactRecord,
    trajectory: &'a RuntimeV2Trajectory,
    evidence: &'a RuntimeV2Evidence,
}

#[derive(Clone, Debug)]
struct EngineResult {
    message: RuntimeV2Message,
    replayed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct OperationBinding {
    instance_id: String,
    session_id: String,
    lease_id: String,
    lease_epoch: u64,
    generation: u64,
    action: RuntimeV2Action,
}

impl OperationBinding {
    fn from_message(message: &RuntimeV2Message) -> Result<Self, RuntimeV2Error> {
        let action = message
            .action()
            .cloned()
            .ok_or(RuntimeV2Error::Invalid("operation has no action"))?;
        let operation_kind = matches!(
            message.kind(),
            RuntimeV2Kind::ActionRequest | RuntimeV2Kind::ReconcileRequest
        );
        if !operation_kind {
            return Err(RuntimeV2Error::Invalid(
                "operation binding requires an action or reconcile request",
            ));
        }
        Ok(Self {
            instance_id: message.instance_id().to_owned(),
            session_id: message.session_id().to_owned(),
            lease_id: message.lease_id().to_owned(),
            lease_epoch: message.lease_epoch(),
            generation: message.generation(),
            action,
        })
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
enum StoredOperation {
    Admission {
        binding: OperationBinding,
        operation_id: RuntimeV2OperationId,
        accepted: RuntimeV2Message,
        settled: Option<RuntimeV2Message>,
        applied: bool,
    },
    Terminal {
        binding: OperationBinding,
        result: RuntimeV2Message,
    },
}

#[derive(Clone, Debug)]
struct FakeRuntimeV2 {
    context: RuntimeV2Context,
    observation: RuntimeV2Observation,
    mutation_count: u16,
    operations: BTreeMap<RuntimeV2OperationId, StoredOperation>,
}
