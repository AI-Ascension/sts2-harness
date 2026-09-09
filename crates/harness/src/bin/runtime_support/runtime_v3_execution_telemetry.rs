// SPDX-License-Identifier: MIT

use super::super::super::runtime_v3_telemetry::{CleanupStatus, GameOutcome, TelemetryStage};
use super::super::recording;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CombatDemoTelemetry {
    pub(super) game_outcome: GameOutcome,
    pub(super) terminal_stage: TelemetryStage,
    pub(super) cleanup_status: CleanupStatus,
}

pub(super) fn combat_demo_telemetry(
    terminal_stage: Option<sts2_harness::EpisodeStage>,
    workflow_cleanup: CleanupStatus,
    provider_close_succeeded: bool,
    store_close_succeeded: bool,
) -> CombatDemoTelemetry {
    let (game_outcome, terminal_stage) = match terminal_stage {
        Some(stage) => (recording::game_outcome(stage), TelemetryStage::from(stage)),
        // Without a terminal observation the host has not established a game result. The
        // durable workflow may quarantine this case as interrupted-unknown, but telemetry must
        // preserve that result as unavailable rather than manufacture success or defeat.
        None => (GameOutcome::Unavailable, TelemetryStage::Unknown),
    };
    let cleanup_status = if workflow_cleanup == CleanupStatus::Clean
        && provider_close_succeeded
        && store_close_succeeded
    {
        CleanupStatus::Clean
    } else {
        CleanupStatus::Failed
    };
    CombatDemoTelemetry {
        game_outcome,
        terminal_stage,
        cleanup_status,
    }
}
