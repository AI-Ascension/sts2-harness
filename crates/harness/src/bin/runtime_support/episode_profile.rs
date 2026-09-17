// SPDX-License-Identifier: MIT

//! Harness-side consumer of the gateway's negotiated repeated-episode profile.
//!
//! The gateway treats an attached deployment as a single episode: a successful
//! `release` permanently revokes the local lease context, so no later allocation
//! is admitted (AI-Ascension/sts2-gateway#67). A caller that intends to complete
//! one episode and then run a *second* episode against the same deployment must
//! negotiate the gateway-owned profile on the release that completes the first.
//!
//! The negotiation is opt-in and off by default, so a harness run that does not
//! ask for it keeps the byte-identical legacy single-episode behavior. This
//! module owns only the request header and the witness check; the gateway owns
//! the admission semantics.

use serde_json::Value;
use std::collections::BTreeMap;

use super::RuntimeConfig;

/// Gateway-owned profile/capability names. These are the accepted contract; the
/// harness must not invent a profile value of its own.
pub(crate) const EPISODE_PROFILE_NAME: &str = "repeated-episode-lease-v1";
pub(crate) const EPISODE_PROFILE_CAPABILITY: &str = "sts2-gateway/repeated-episode-lease-v1";
pub(crate) const EPISODE_PROFILE_HEADER: &str = "x-sts2-episode-profile";
pub(crate) const EPISODE_PROFILE_SCHEMA_DIGEST: &str =
    "f3a04bab61ce4898eda0fa88cb546441493e49eef19b1e5e0841a3b4ef7c4331";

/// Adds the negotiated-profile header to a release request when the run opted
/// in. A run that did not opt in sends nothing and the gateway keeps its
/// permanent-revocation default.
pub(crate) fn apply_episode_profile(headers: &mut BTreeMap<String, String>, negotiated: bool) {
    if negotiated {
        headers.insert(
            String::from(EPISODE_PROFILE_HEADER),
            String::from(EPISODE_PROFILE_NAME),
        );
    }
}

/// Verifies the gateway's witness when this run negotiated the profile.
///
/// A profiled release must report the accepted capability, the schema digest
/// this build was written against, and the completed epoch. An opt-in run that
/// receives no witness has not actually armed the profile, so a following
/// episode would be admitted only by luck; that is treated as a failure rather
/// than silently degrading to the single-episode default.
pub(crate) fn confirm_episode_profile_witness(
    response: &Value,
    negotiated: bool,
    lease_epoch: u64,
) -> Result<(), String> {
    if !negotiated {
        return Ok(());
    }
    let witness = response.get("episode_profile").ok_or_else(|| {
        String::from("gateway release did not report the negotiated episode profile")
    })?;
    if witness["profile"].as_str() != Some(EPISODE_PROFILE_NAME) {
        return Err(String::from(
            "gateway release reported an unexpected episode profile",
        ));
    }
    if witness["capability"].as_str() != Some(EPISODE_PROFILE_CAPABILITY) {
        return Err(String::from(
            "gateway release reported an unexpected episode profile capability",
        ));
    }
    if witness["schema_digest"].as_str() != Some(EPISODE_PROFILE_SCHEMA_DIGEST) {
        return Err(String::from(
            "gateway release reported an unexpected episode profile schema digest",
        ));
    }
    if witness["released_epoch"].as_u64() != Some(lease_epoch) {
        return Err(String::from(
            "gateway release reported an unexpected completed episode epoch",
        ));
    }
    Ok(())
}

/// Release headers for one release call. `completes_episode` gates the opt-in
/// repeated-episode negotiation so cleanup and failure paths keep the gateway's
/// default permanent revocation.
pub(crate) fn release_headers(
    config: &RuntimeConfig,
    completes_episode: bool,
) -> BTreeMap<String, String> {
    let mut headers = super::identity_headers(config, &super::release_correlation());
    apply_episode_profile(&mut headers, config.episode_profile && completes_episode);
    headers
}

/// Confirms an authoritative release and, when this release completed an
/// episode under the opt-in profile, requires the gateway's exact witness.
pub(crate) fn confirm_release(
    response: Result<Value, String>,
    config: &RuntimeConfig,
    completes_episode: bool,
) -> Result<(), String> {
    let value = response?;
    if value["status"] != "released" {
        return Err(String::from("gateway did not confirm lease release"));
    }
    confirm_episode_profile_witness(
        &value,
        config.episode_profile && completes_episode,
        config.lease_epoch,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_opt_out_run_sends_no_profile_header() {
        let mut headers = BTreeMap::new();
        apply_episode_profile(&mut headers, false);
        assert!(headers.is_empty());
    }

    #[test]
    fn an_opt_in_run_negotiates_the_accepted_profile() {
        let mut headers = BTreeMap::new();
        apply_episode_profile(&mut headers, true);
        assert_eq!(headers.len(), 1);
        assert_eq!(
            headers.get(EPISODE_PROFILE_HEADER).map(String::as_str),
            Some(EPISODE_PROFILE_NAME)
        );
    }

    #[test]
    fn a_witness_is_ignored_when_the_run_did_not_opt_in() {
        let response = json!({ "status": "released" });
        assert!(confirm_episode_profile_witness(&response, false, 3).is_ok());
    }

    #[test]
    fn an_opt_in_run_requires_the_exact_accepted_witness() {
        let response = json!({
            "status": "released",
            "episode_profile": {
                "profile": EPISODE_PROFILE_NAME,
                "capability": EPISODE_PROFILE_CAPABILITY,
                "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST,
                "released_epoch": 3,
            }
        });
        assert!(confirm_episode_profile_witness(&response, true, 3).is_ok());
    }

    #[test]
    fn an_opt_in_run_rejects_a_missing_or_wrong_witness() {
        let missing = json!({ "status": "released" });
        assert!(confirm_episode_profile_witness(&missing, true, 3).is_err());

        for witness in [
            json!({ "profile": "single-episode", "capability": EPISODE_PROFILE_CAPABILITY,
                    "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST, "released_epoch": 3 }),
            json!({ "profile": EPISODE_PROFILE_NAME, "capability": "sts2-gateway/other",
                    "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST, "released_epoch": 3 }),
            json!({ "profile": EPISODE_PROFILE_NAME, "capability": EPISODE_PROFILE_CAPABILITY,
                    "schema_digest": "0000000000000000000000000000000000000000000000000000000000000000",
                    "released_epoch": 3 }),
            json!({ "profile": EPISODE_PROFILE_NAME, "capability": EPISODE_PROFILE_CAPABILITY,
                    "schema_digest": EPISODE_PROFILE_SCHEMA_DIGEST, "released_epoch": 2 }),
        ] {
            let response = json!({ "status": "released", "episode_profile": witness });
            assert!(
                confirm_episode_profile_witness(&response, true, 3).is_err(),
                "an unexpected witness must be rejected: {witness}"
            );
        }
    }
}
