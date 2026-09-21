// SPDX-License-Identifier: MIT

//! Typed provider-kind admission.
//!
//! `STS2_PROVIDER_KIND` names the provider lane a run may be admitted on. It was previously read as
//! a bare string in one place and compared against one spelling in another, so a name nobody
//! implemented was not refused at all: it fell through the local-bridge branch and ran under the
//! reviewed source revision as though the operator had named that lane deliberately. The two
//! properties that actually decide what a lane may do — whether its provider is a locally launched
//! bridge, and whether it declares the live-episode capability — are now data on the type, and an
//! unknown name is refused while the runtime is still assembling settings, before it opens a
//! durable store, a gateway connection, an MCP session or a provider.
//!
//! `synthetic` and `exo` are deliberately not local bridges. The first is the operator's own name
//! for the raw-wire probe lane, and the second is the packaged Exo executor, whose identity the
//! reviewed envelope binds rather than a digest the operator supplies beside the declaration.

/// A provider lane this runtime can be admitted on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProviderKind {
    /// The Astra bridge: a local executable, and the lane live episodes were once limited to.
    OpenAstra,
    /// The Ollama bridge: a local executable, never admitted for a live episode.
    Ollama,
    /// The System One bridge: a local executable, never admitted for a live episode.
    TypesafeJev,
    /// The packaged Exo executor, admitted through the reviewed envelope.
    Exo,
    /// The operator's raw-wire probe lane: the reviewed source revision, no bridge of its own.
    Synthetic,
}

impl ProviderKind {
    /// The spelling this kind is declared with.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::OpenAstra => "openai-astra",
            Self::Ollama => "ollama",
            Self::TypesafeJev => "typesafe-jev",
            Self::Exo => "exo",
            Self::Synthetic => "synthetic",
        }
    }

    /// Whether `STS2_EXO_BRIDGE_BINARY` is this lane's provider, digest-pinned by the operator.
    pub(super) fn is_local_bridge(self) -> bool {
        matches!(self, Self::OpenAstra | Self::Ollama | Self::TypesafeJev)
    }

    /// Whether this kind declares the live-episode capability.
    pub(super) fn admits_live_episode(self) -> bool {
        matches!(self, Self::OpenAstra | Self::Exo)
    }

    /// Whether that capability is only admitted once the reviewed envelope validated it.
    ///
    /// The Astra lane's live capability is an executable digest and an exact argument vector, which
    /// the operator supplies and this module checks. The Exo lane's capability is the reviewed
    /// descriptor, whose package, extension, model, tool and configuration identities only the
    /// envelope inspects, so the raw-wire acknowledgement cannot stand in for it.
    pub(super) fn requires_reviewed_envelope(self) -> bool {
        matches!(self, Self::Exo)
    }
}

/// The declared provider kind, or `None` when the operator named none.
///
/// A name this runtime does not implement is refused rather than ignored: ignoring it is how the
/// reviewed Exo lane was reachable without the run ever saying which lane it was on.
pub(super) fn declared() -> Result<Option<ProviderKind>, String> {
    match super::optional("STS2_PROVIDER_KIND")?.as_deref() {
        None => Ok(None),
        Some(name) => parse(name).map(Some),
    }
}

fn parse(name: &str) -> Result<ProviderKind, String> {
    match name {
        "openai-astra" => Ok(ProviderKind::OpenAstra),
        "ollama" => Ok(ProviderKind::Ollama),
        "typesafe-jev" => Ok(ProviderKind::TypesafeJev),
        "exo" => Ok(ProviderKind::Exo),
        "synthetic" => Ok(ProviderKind::Synthetic),
        _ => Err(format!(
            "STS2_PROVIDER_KIND {name} is not a provider kind this runtime implements"
        )),
    }
}

/// Admits the identity and argument vector of a locally launched provider bridge.
///
/// This runs while settings are still being assembled. A local bridge is an executable this runtime
/// launches, so its exact bytes are hashed here and its declared argument vector is re-parsed with
/// the parser the bridge itself uses, which is what keeps admission and the executable from holding
/// two opinions about a valid invocation.
pub(super) fn verify_bridge(
    kind: Option<ProviderKind>,
    revision: &str,
    bounded_mode: bool,
) -> Result<(), String> {
    let local_bridge = kind.is_some_and(ProviderKind::is_local_bridge);
    if local_bridge && (revision.len() != 64 || !bounded_mode) {
        return Err(String::from(
            "Local provider requires the bridge SHA256 and explicit combat, campaign, or live episode mode",
        ));
    }
    if local_bridge {
        use std::io::Read;
        let file = std::fs::File::open(super::required("STS2_EXO_BRIDGE_BINARY")?)
            .map_err(|_| String::from("cannot open provider bridge for digest verification"))?;
        let mut bytes = Vec::new();
        file.take(128 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| String::from("cannot hash provider bridge"))?;
        if bytes.len() > 128 * 1024 * 1024
            || sts2_harness::sha256_hex(&bytes) != revision
            || !super::local_bridge::arguments_allowed(
                kind.map(ProviderKind::name),
                &super::string_list("STS2_EXO_BRIDGE_ARGS_JSON")?,
            )
        {
            return Err(String::from(
                "Provider bridge digest or arguments do not match",
            ));
        }
    }
    if !local_bridge && revision != super::REVIEWED_EXO_REVISION {
        return Err(String::from(
            "STS2_EXO_REVISION is not the reviewed Exo revision",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_v3_settings_provider_tests.rs"]
mod tests;
