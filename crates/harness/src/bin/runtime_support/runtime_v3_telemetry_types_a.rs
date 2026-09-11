// SPDX-License-Identifier: MIT

// The runtime-v3 telemetry boundary.
//
// This module deliberately lives beside the executable adapter. It accepts only
// finite enums, bounded scalars, and digests of host/provider identities. The
// worker owns the loopback socket; gameplay calls only enqueue into one bounded
// FIFO channel and never perform network I/O.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{ActionKind, DispatchStatus, EpisodeObservation, EpisodeStage, PolicyError};

const ENDPOINT: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 14318);
const OTLP_PATH: &str = "/v1/traces";
// The queue keeps the historical normal/critical budget as one FIFO total.
// This preserves bounded admission while making sequence order authoritative.
const NORMAL_QUEUE_CAPACITY: usize = 256;
const CRITICAL_QUEUE_CAPACITY: usize = 8;
const MAX_BATCH: usize = 64;
const MAX_BODY_BYTES: usize = 512 * 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_millis(500);
const MAX_RESPONSE_BYTES: usize = 16 * 1024;
const RUN_ID_DIGEST_DOMAIN: &str = "run";
const EPISODE_ID_DIGEST_DOMAIN: &str = "episode";
const TRAJECTORY_ID_DIGEST_DOMAIN: &str = "trajectory";
const TRACE_ID_DIGEST_DOMAIN: &str = "trace";
const INSTANCE_ID_DIGEST_DOMAIN: &str = "instance";
const SESSION_ID_DIGEST_DOMAIN: &str = "session";

#[derive(Clone, Eq, PartialEq)]
pub struct TelemetryContext {
    // These values are the only harness lineage identities retained for
    // serialization. Their field names intentionally remain the canonical
    // OTLP attribute names; each value is a domain-separated SHA-256 digest.
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub trace_id: String,
    pub instance_id: String,
    pub session_id: String,
    pub runtime_profile: String,
    pub schema_version: String,
    pub provider_revision_digest: String,
    // This is retained only to derive the OTLP trace/span topology. It is
    // never added to an OTLP attribute.
    trace_lineage_id: String,
}

pub struct TelemetryContextInput<'a> {
    pub run_id: &'a str,
    pub episode_id: &'a str,
    pub trajectory_id: &'a str,
    pub trace_id: &'a str,
    pub instance_id: &'a str,
    pub session_id: &'a str,
    pub runtime_profile: &'a str,
    pub provider_revision: &'a str,
}

impl TelemetryContext {
    pub fn new(input: TelemetryContextInput<'_>) -> Result<Self, String> {
        let TelemetryContextInput {
            run_id,
            episode_id,
            trajectory_id,
            trace_id,
            instance_id,
            session_id,
            runtime_profile,
            provider_revision,
        } = input;
        for (name, value) in [
            ("run_id", run_id),
            ("episode_id", episode_id),
            ("trajectory_id", trajectory_id),
            ("trace_id", trace_id),
            ("instance_id", instance_id),
            ("session_id", session_id),
            ("runtime_profile", runtime_profile),
        ] {
            if !safe_identity(value) {
                return Err(format!("telemetry {name} is empty, unsafe, or oversized"));
            }
        }
        let identities = [run_id, episode_id, trajectory_id, trace_id];
        if identities
            .iter()
            .enumerate()
            .any(|(index, value)| identities[..index].contains(value))
        {
            return Err(String::from(
                "telemetry run, episode, trajectory, and trace identities must be distinct",
            ));
        }
        if !valid_revision(provider_revision) {
            return Err(String::from("telemetry provider revision is invalid"));
        }
        Ok(Self {
            run_id: digest(RUN_ID_DIGEST_DOMAIN, run_id),
            episode_id: digest(EPISODE_ID_DIGEST_DOMAIN, episode_id),
            trajectory_id: digest(TRAJECTORY_ID_DIGEST_DOMAIN, trajectory_id),
            trace_id: digest(TRACE_ID_DIGEST_DOMAIN, trace_id),
            instance_id: digest(INSTANCE_ID_DIGEST_DOMAIN, instance_id),
            session_id: digest(SESSION_ID_DIGEST_DOMAIN, session_id),
            runtime_profile: runtime_profile.to_owned(),
            schema_version: String::from("runtime-v3-telemetry-1"),
            provider_revision_digest: digest("provider-revision", provider_revision),
            trace_lineage_id: trace_id.to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionKind {
    Action,
    Plan,
    Wait,
    Reobserve,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryActionKind {
    StartRun,
    SelectCharacter,
    SelectMapNode,
    PlayCard,
    UsePotion,
    EndTurn,
    ChooseReward,
    SkipReward,
    ShopPurchase,
    ShopRemove,
    Rest,
    RestOption,
    Smith,
    EventChoice,
    SelectCard,
    ConfirmVictory,
    SaveQuit,
    Proceed,
    ConfirmSelection,
    CancelSelection,
    SelectPlayer,
}

impl TelemetryActionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::StartRun => "start_run",
            Self::SelectCharacter => "select_character",
            Self::SelectMapNode => "select_map_node",
            Self::PlayCard => "play_card",
            Self::UsePotion => "use_potion",
            Self::EndTurn => "end_turn",
            Self::ChooseReward => "choose_reward",
            Self::SkipReward => "skip_reward",
            Self::ShopPurchase => "shop_purchase",
            Self::ShopRemove => "shop_remove",
            Self::Rest => "rest",
            Self::RestOption => "rest_option",
            Self::Smith => "smith",
            Self::EventChoice => "event_choice",
            Self::SelectCard => "select_card",
            Self::ConfirmVictory => "confirm_victory",
            Self::SaveQuit => "save_quit",
            Self::Proceed => "proceed",
            Self::ConfirmSelection => "confirm_selection",
            Self::CancelSelection => "cancel_selection",
            Self::SelectPlayer => "select_player",
        }
    }
}

impl From<ActionKind> for TelemetryActionKind {
    fn from(kind: ActionKind) -> Self {
        match kind {
            ActionKind::StartRun => Self::StartRun,
            ActionKind::SelectCharacter => Self::SelectCharacter,
            ActionKind::SelectMapNode => Self::SelectMapNode,
            ActionKind::PlayCard => Self::PlayCard,
            ActionKind::UsePotion => Self::UsePotion,
            ActionKind::EndTurn => Self::EndTurn,
            ActionKind::ChooseReward => Self::ChooseReward,
            ActionKind::SkipReward => Self::SkipReward,
            ActionKind::ShopPurchase => Self::ShopPurchase,
            ActionKind::ShopRemove => Self::ShopRemove,
            ActionKind::Rest => Self::Rest,
            ActionKind::RestOption => Self::RestOption,
            ActionKind::Smith => Self::Smith,
            ActionKind::EventChoice => Self::EventChoice,
            ActionKind::SelectCard => Self::SelectCard,
            ActionKind::ConfirmVictory => Self::ConfirmVictory,
            ActionKind::SaveQuit => Self::SaveQuit,
            ActionKind::Proceed => Self::Proceed,
            ActionKind::ConfirmSelection => Self::ConfirmSelection,
            ActionKind::CancelSelection => Self::CancelSelection,
            ActionKind::SelectPlayer => Self::SelectPlayer,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetryStage {
    Setup,
    Map,
    Combat,
    Reward,
    Shop,
    Event,
    Rest,
    Selection,
    Victory,
    Defeat,
    Recovery,
    Unknown,
}

impl TelemetryStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Map => "map",
            Self::Combat => "combat",
            Self::Reward => "reward",
            Self::Shop => "shop",
            Self::Event => "event",
            Self::Rest => "rest",
            Self::Selection => "selection",
            Self::Victory => "victory",
            Self::Defeat => "defeat",
            Self::Recovery => "recovery",
            Self::Unknown => "unknown",
        }
    }
}

impl From<EpisodeStage> for TelemetryStage {
    fn from(stage: EpisodeStage) -> Self {
        match stage {
            EpisodeStage::Setup => Self::Setup,
            EpisodeStage::Map => Self::Map,
            EpisodeStage::Combat => Self::Combat,
            EpisodeStage::Reward => Self::Reward,
            EpisodeStage::Shop => Self::Shop,
            EpisodeStage::Event => Self::Event,
            EpisodeStage::Rest => Self::Rest,
            EpisodeStage::Selection => Self::Selection,
            EpisodeStage::Victory => Self::Victory,
            EpisodeStage::Defeat => Self::Defeat,
            EpisodeStage::Recovery => Self::Recovery,
            EpisodeStage::Unknown => Self::Unknown,
        }
    }
}


include!("runtime_v3_telemetry_types_a_tail.rs");
