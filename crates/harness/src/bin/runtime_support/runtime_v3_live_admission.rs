// SPDX-License-Identifier: MIT

//! The admitted live-episode mode.
//!
//! Live-episode behavior is the bounded replay/receipt stream together with the operator
//! diagnostics that follow one. Every point of use used to re-read the raw `STS2_LIVE_EPISODE`
//! variable, which cannot tell a lane admission granted from one it refused — the variable is
//! ambient to the process, not to the decision — and which ties behavior to a spelling rather than
//! to the capability admission actually granted. A second live-capable provider kind would
//! therefore have switched live recording on for every lane that happened to inherit the variable.
//!
//! The mode is resolved once, from the provider kind admission accepted, and installed by settings
//! assembly after the whole run is admitted. Points of use read the installed decision, so the
//! replay stream and the live diagnostics follow the same admission the provider did.

use std::sync::OnceLock;

use sts2_harness::exo_admission::ExoAdmissionMode;

use super::provider::ProviderKind;

const LIVE_EPISODE: &str = "STS2_LIVE_EPISODE";

/// The mode a run was admitted for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LiveMode {
    /// No live episode: no bounded replay stream and no live-only diagnostics.
    Standard,
    /// An admitted live episode.
    LiveEpisode,
}

static ADMITTED: OnceLock<LiveMode> = OnceLock::new();

/// Whether the operator declared a live episode, read in this one place.
///
/// Only the exact spelling `true` declares one, which is the long-standing meaning of the variable:
/// any other spelling leaves the run standard rather than being read generously.
pub(crate) fn declared() -> Result<bool, String> {
    Ok(super::optional(LIVE_EPISODE)?.as_deref() == Some("true"))
}

/// The mode an admitted provider kind runs with.
///
/// A declared live episode is admitted by capability, not by the name of the kind: the kind must
/// declare the capability, the operator must have named a kind at all, and a kind whose capability
/// is bound by the reviewed envelope is refused when only the raw-wire acknowledgement was given.
pub(crate) fn resolve(
    kind: Option<ProviderKind>,
    declared: bool,
    admission: ExoAdmissionMode,
) -> Result<LiveMode, String> {
    if !declared {
        return Ok(LiveMode::Standard);
    }
    let Some(kind) = kind else {
        return Err(format!(
            "{LIVE_EPISODE} requires STS2_PROVIDER_KIND to name a kind that declares live-episode capability"
        ));
    };
    if !kind.admits_live_episode() {
        return Err(format!(
            "provider kind {} does not declare live-episode capability",
            kind.name()
        ));
    }
    if kind.requires_reviewed_envelope() && admission != ExoAdmissionMode::Enveloped {
        return Err(format!(
            "provider kind {} is admitted for a live episode only through the reviewed envelope; the raw-wire acknowledgement does not inspect its capability descriptor",
            kind.name()
        ));
    }
    Ok(LiveMode::LiveEpisode)
}

/// The mode for a lane whose live capability is backed by its own pinned identity.
///
/// The game-information lookup lane runs no decision provider: its provider is the lookup agent,
/// pinned by its own digest and owner configuration, so `STS2_PROVIDER_KIND` names nothing on that
/// path and the declaration is the whole of its live-episode claim.
pub(crate) fn resolve_declared(declared: bool) -> LiveMode {
    if declared {
        LiveMode::LiveEpisode
    } else {
        LiveMode::Standard
    }
}

/// Installs the admitted mode.
///
/// Settings assembly calls this once the run is fully admitted, so a run refused earlier cannot
/// have switched live recording on. Assembling settings again for the same mode is allowed; a
/// second, contradicting resolution is refused rather than allowed to re-decide the process.
pub(crate) fn install(mode: LiveMode) -> Result<(), String> {
    // `get_or_init` installs the first resolution and returns whichever one the process holds.
    // `set` cannot be used instead: it reports the value it was handed rather than the value
    // already in the cell, which cannot tell a repeated resolution from a contradicting one.
    let installed = *ADMITTED.get_or_init(|| mode);
    if installed == mode {
        return Ok(());
    }
    Err(format!(
        "the admitted live mode is already {installed:?} and cannot also be {mode:?}"
    ))
}

/// Whether the admitted mode is a live episode.
///
/// Before installation — and therefore for any build that reaches a live-only branch without
/// passing through admission — this is `Standard` rather than whatever the ambient variable says.
pub(crate) fn admitted_live_episode() -> bool {
    ADMITTED.get() == Some(&LiveMode::LiveEpisode)
}

#[cfg(test)]
#[path = "runtime_v3_live_admission_tests.rs"]
mod tests;
