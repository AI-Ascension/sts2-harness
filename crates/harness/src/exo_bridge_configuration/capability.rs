// SPDX-License-Identifier: MIT

//! Machine-checkable capability advertisement for the one-shot Exo bridge.
//!
//! The advertisement and the runtime guard are the same data: [`unsupported_profile_axis`] is what
//! the shipped entry points call, and [`capability_fields`] publishes the exact sets it enforces.
//! Splitting this out of the configuration module keeps each file within the repository size rule
//! without duplicating the vocabulary.

use crate::{EXO_SOURCE_REVISION, ExoDecisionRequest};
use serde_json::{Value, json};

/// Profiles the reviewed extension in this build actually implements.
pub const SUPPORTED_PROFILES: [&str; 1] = ["standard"];
/// Profiles the pinned upstream source names but this build rejects before any inference.
pub const UNSUPPORTED_PROFILES: [&str; 2] = ["map", "expert"];
/// Context modes implemented by this one-shot build.
pub const SUPPORTED_CONTEXT_MODES: [&str; 1] = ["fresh"];
/// Terminal decisions this build returns to the host.
pub const SUPPORTED_DECISIONS: [&str; 4] = ["action", "plan", "wait", "reobserve"];
/// Terminal decisions the contract can parse but this build refuses to dispatch.
pub const UNSUPPORTED_DECISIONS: [&str; 1] = ["recovery"];
/// Terminal decisions the lookup relay returns: it is terminal on an action id only.
pub const LOOKUP_SUPPORTED_DECISIONS: [&str; 1] = ["action_id"];
/// Terminal decisions the lookup relay cannot dispatch, including the one-shot decision kinds.
pub const LOOKUP_UNSUPPORTED_DECISIONS: [&str; 5] =
    ["action", "plan", "wait", "reobserve", "recovery"];
/// Single fail-closed rejection code for every unsupported request-profile axis.
pub const UNSUPPORTED_PROFILE_CODE: &str = "exo_bridge_unsupported_profile";
/// Single fail-closed rejection code for the unsupported recovery decision.
pub const UNSUPPORTED_RECOVERY_CODE: &str = "exo_bridge_unsupported_recovery";

/// A request axis that the shipped one-shot build deliberately does not implement.
///
/// The axis name is published in `--describe` so a caller can pre-check support instead of
/// inferring it from a rejection code that is intentionally identical for every axis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedProfileAxis {
    /// `provider_revision` is not the reviewed source revision.
    Revision,
    /// The request carries `map_context`.
    Map,
    /// The request carries `management_profile`/`management_context`.
    Management,
    /// The observation declares an expert `protocol_version`.
    Expert,
}

impl UnsupportedProfileAxis {
    /// Stable machine-readable axis name shared with the capability advertisement.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Revision => "revision",
            Self::Map => "map",
            Self::Management => "management",
            Self::Expert => "expert",
        }
    }
}

/// Classifies the first unsupported profile axis of an already schema-validated request.
///
/// Returns `None` only when every axis this build implements is present. The ordering is fixed, so
/// a request that is unsupported for several reasons always reports the same axis. Both the
/// one-shot entry point and the lookup relay call this, so the guard cannot drift from the
/// advertisement.
#[must_use]
pub fn unsupported_profile_axis(request: &ExoDecisionRequest) -> Option<UnsupportedProfileAxis> {
    if request.provider_revision != EXO_SOURCE_REVISION {
        return Some(UnsupportedProfileAxis::Revision);
    }
    if request.map_context.is_some() {
        return Some(UnsupportedProfileAxis::Map);
    }
    if request.management_profile.is_some() {
        return Some(UnsupportedProfileAxis::Management);
    }
    if request.observation.get("protocol_version").is_some() {
        return Some(UnsupportedProfileAxis::Expert);
    }
    None
}

/// The only model binding the synthetic smoke mode may use.
pub const SYNTHETIC_MODEL: &str = "o3-pro";
/// The only reviewed external provider route.
pub const PROVIDER_ENDPOINT: &str = "https://api.openai.com/v1";
/// Loopback prefix that keeps a synthetic run on the machine.
pub const SYNTHETIC_ENDPOINT_PREFIX: &str = "http://127.0.0.1:";

/// Whether a synthetic run stays on a literal loopback port with the synthetic-only model.
///
/// This is the property that makes the smoke test unable to reach a real configured external
/// provider: an external HTTPS endpoint, a non-loopback host, port `0` and any other model are all
/// refused. It is a pure predicate so the shipped guard and the tests cannot disagree.
#[must_use]
pub fn synthetic_route_admitted(endpoint: &str, model: &str) -> bool {
    endpoint
        .strip_prefix(SYNTHETIC_ENDPOINT_PREFIX)
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port != 0)
        && model == SYNTHETIC_MODEL
}

/// Whether a non-synthetic run uses the single reviewed OpenAI HTTPS route.
#[must_use]
pub fn provider_route_admitted(endpoint: &str) -> bool {
    endpoint == PROVIDER_ENDPOINT
}

/// Machine-checkable capability fields shared by every `--describe`-style advertisement.
///
/// The advertised sets are the exact sets the runtime guard enforces; a test asserts the two
/// cannot disagree. The decision sets are parameters rather than the one-shot constants because
/// the lookup relay is terminal on `action_id` only: reusing the one-shot set would advertise
/// `plan`/`wait`/`reobserve` as supported on an entry point that cannot dispatch them.
#[must_use]
pub fn capability_fields(
    supported_decisions: &[&str],
    unsupported_decisions: &[&str],
) -> serde_json::Map<String, Value> {
    let profile_support = SUPPORTED_PROFILES
        .iter()
        .chain(UNSUPPORTED_PROFILES.iter())
        .map(|profile| {
            let state = if SUPPORTED_PROFILES.contains(profile) {
                "supported"
            } else {
                "unsupported"
            };
            (profile.to_string(), json!(state))
        })
        .collect::<serde_json::Map<_, _>>();
    let decision_support = supported_decisions
        .iter()
        .map(|decision| (decision.to_string(), json!("supported")))
        .chain(
            unsupported_decisions
                .iter()
                .map(|decision| (decision.to_string(), json!("unsupported"))),
        )
        .collect::<serde_json::Map<_, _>>();
    let mut fields = serde_json::Map::new();
    fields.insert("profiles".to_owned(), json!(SUPPORTED_PROFILES));
    fields.insert("profile_support".to_owned(), json!(profile_support));
    fields.insert("context_modes".to_owned(), json!(SUPPORTED_CONTEXT_MODES));
    fields.insert("decisions".to_owned(), json!(supported_decisions));
    fields.insert("decision_support".to_owned(), json!(decision_support));
    fields.insert(
        "unsupported_profile_code".to_owned(),
        json!(UNSUPPORTED_PROFILE_CODE),
    );
    fields.insert(
        "unsupported_recovery_code".to_owned(),
        json!(UNSUPPORTED_RECOVERY_CODE),
    );
    fields
}
