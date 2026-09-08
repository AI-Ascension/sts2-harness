// SPDX-License-Identifier: MIT

use std::io::Read;

#[cfg(test)]
use serde_json::Value;
use serde_json::json;
use sha2::{Digest, Sha256};
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeObservation, EpisodeRunner,
    EpisodeRunnerConfig, EpisodeStage, PolicyError,
};

use super::{RuntimeV3Port, recording, wire};

#[path = "runtime_v3_replay_trace.rs"]
mod trace;
#[cfg(test)]
use trace::canonical;
use trace::{ReplayTrace, action_payload};
#[path = "runtime_v3_replay_cards.rs"]
mod cards;
use cards::CardBindings;

const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[path = "runtime_v3_replay_digest.rs"]
mod digest;
use digest::digest_value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReplayOutcome {
    Terminal(EpisodeStage),
    PrefixVerified,
}

pub(super) fn run(
    port: &mut RuntimeV3Port,
    config: &EpisodeRunnerConfig,
    path: &str,
) -> Result<ReplayOutcome, String> {
    // Validate the complete source before the runner allocates a host lease.
    let file = std::fs::File::open(path).map_err(|_| "cannot open episode replay")?;
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read episode replay")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("episode replay exceeds byte bound".into());
    }
    let prefix = match std::env::var("STS2_REPLAY_PREFIX").as_deref() {
        Ok("true") => true,
        Ok("false") | Err(std::env::VarError::NotPresent) => false,
        _ => return Err("STS2_REPLAY_PREFIX must be true or false".into()),
    };
    let trace = if prefix {
        ReplayTrace::parse_mode(&bytes, true)?
    } else {
        ReplayTrace::parse(&bytes)?
    };
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let mut source = ReplaySource::new(trace);
    println!(
        "{}",
        json!({"event":"episode_replay_started", "source_sha256":digest,
        "recorded_actions":source.trace.records.len(), "provider_calls":0,
        "skipped_rejected_attempts":source.trace.rejected_attempts})
    );
    let result = EpisodeRunner::new(config.clone()).run(port, &mut source);
    if matches!(
        result,
        Err(sts2_harness::EpisodeRunnerError::StoppedByRecovery)
    ) && source.prefix_verified
    {
        println!(
            "{}",
            json!({"event":"episode_replay_prefix_verified",
            "source_sha256":digest,"replayed_actions":source.cursor,"provider_calls":0,
            "skipped_rejected_attempts":source.trace.rejected_attempts})
        );
        return Ok(ReplayOutcome::PrefixVerified);
    }
    let report = result.map_err(|error| {
        format!(
            "episode replay failed: {}",
            source
                .failure
                .map(str::to_owned)
                .unwrap_or_else(|| error_category(&error))
        )
    })?;
    source.finish(report.final_observation())?;
    recording::complete(&report, &port.telemetry);
    println!(
        "{}",
        json!({"event":"episode_replay_verified", "source_sha256":digest,
        "replayed_actions":source.cursor, "provider_calls":0,
        "skipped_rejected_attempts":source.trace.rejected_attempts,
        "terminal_stage":wire::stage_name(report.terminal_stage())})
    );
    Ok(ReplayOutcome::Terminal(report.terminal_stage()))
}

fn error_category(error: &sts2_harness::EpisodeRunnerError) -> String {
    match error {
        sts2_harness::EpisodeRunnerError::UncertainMutation => {
            "host mutation did not settle".into()
        }
        sts2_harness::EpisodeRunnerError::StepLimitExceeded => "step budget exhausted".into(),
        // Runner Display implementations expose typed categories, not provider content.
        _ => error.to_string(),
    }
}

struct ReplaySource {
    trace: ReplayTrace,
    cursor: usize,
    awaiting: bool,
    failure: Option<&'static str>,
    prefix_verified: bool,
    prefix_observation: Option<serde_json::Value>,
    observation_waits: u8,
    cards: CardBindings,
}

impl ReplaySource {
    fn new(trace: ReplayTrace) -> Self {
        Self {
            trace,
            cursor: 0,
            awaiting: false,
            failure: None,
            prefix_verified: false,
            prefix_observation: None,
            observation_waits: 0,
            cards: CardBindings::default(),
        }
    }

    fn reject(&mut self, reason: &'static str) -> Result<Decision, PolicyError> {
        self.failure = Some(reason);
        Err(PolicyError::MalformedDecision)
    }

    fn finish(&self, observation: &EpisodeObservation) -> Result<(), String> {
        if self.awaiting || self.failure.is_some() || self.cursor != self.trace.records.len() {
            return Err("episode ended before all replay actions settled".into());
        }
        if self
            .cards
            .reconcile(&self.trace.terminal, observation.fair_play().as_value())
            .is_none()
        {
            return Err("terminal episode replay observation diverged".into());
        }
        Ok(())
    }
}

impl DecisionSource for ReplaySource {
    fn action_completed(&mut self, settled: bool) {
        if self.awaiting && settled {
            self.cursor += 1;
            self.awaiting = false;
        } else {
            self.failure = Some("replay action did not settle");
        }
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        if self.awaiting || self.failure.is_some() {
            return self.reject("replay has an unresolved action");
        }
        let Some(record) = self.trace.records.get(self.cursor) else {
            if self.trace.prefix && self.finish(&input.observation).is_ok() {
                self.prefix_verified = true;
                self.prefix_observation = Some(input.observation.fair_play().as_value().clone());
                return Ok(Decision::Recovery {
                    kind: "stop_episode".into(),
                    operation_id: None,
                    rationale: "Verified recorded prefix checkpoint; release replay lease".into(),
                });
            }
            return self.reject("replay sequence exhausted before terminal state");
        };
        let bindings = self.cards.reconcile(
            &record.observation,
            input.observation.fair_play().as_value(),
        );
        let Some(bindings) = bindings else {
            if self.cursor > 0
                && self.observation_waits < 3
                && record.observation["visible_seed"]
                    == input.observation.fair_play().as_value()["visible_seed"]
            {
                self.observation_waits += 1;
                return Ok(Decision::Wait {
                    rationale: "Await recorded public boundary without dispatching an action"
                        .into(),
                });
            }
            return self.reject("episode replay observation diverged before dispatch");
        };
        self.observation_waits = 0;
        let payload = bindings.translate(&record.payload);
        let matches: Vec<_> = input
            .legal_actions
            .actions()
            .iter()
            .filter(|action| {
                action_payload(input.observation.fair_play().as_value(), action.action_id())
                    == Some(&payload)
            })
            .collect();
        if matches.len() != 1 {
            return self.reject("recorded action is not uniquely legal in the current catalog");
        }
        let action_id = matches[0].action_id().to_owned();
        self.cards = bindings;
        self.awaiting = true;
        println!(
            "{}",
            json!({"event":"replay_decision", "replay_index":self.cursor,
            "observation_digest":digest_value(
                "replay-observation",
                input.observation.fair_play().as_value(),
            ),
            "action_digest":digest_value("replay-action", &payload)})
        );
        Ok(Decision::Action {
            action_id,
            rationale: "Recorded action replay; no model inference".into(),
            confidence: None,
        })
    }
}

#[cfg(test)]
#[path = "runtime_v3_episode_replay_tests.rs"]
mod tests;
