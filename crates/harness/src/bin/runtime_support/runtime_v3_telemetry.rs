// SPDX-License-Identifier: MIT

mod implementation {
    include!("runtime_v3_telemetry_types_a.rs");
    include!("runtime_v3_telemetry_types_extra.rs");
    include!("runtime_v3_telemetry_types_b.rs");
    include!("runtime_v3_telemetry_types_c.rs");
    include!("runtime_v3_telemetry_worker.rs");
    include!("runtime_v3_telemetry_render.rs");
    include!("runtime_v3_telemetry_test_support.rs");
    include!("runtime_v3_telemetry_tests.rs");
    include!("runtime_v3_telemetry_identity_privacy_tests.rs");
}

pub(super) use implementation::{
    CleanupStatus, DecisionKind, FailureCode, GameOutcome, ObservationSource, RecoveryKind,
    RuntimeV3Telemetry, TelemetryContext, TelemetryContextInput, TelemetryHandle, TelemetryStage,
};
