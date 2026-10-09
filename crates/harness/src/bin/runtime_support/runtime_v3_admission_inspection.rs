// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeV3AdmissionMode {
    Enveloped,
    Legacy,
    SyntheticEnvelope,
}

impl RuntimeV3AdmissionMode {
    pub(super) const fn supports_profiles(self) -> bool {
        matches!(self, Self::Enveloped | Self::SyntheticEnvelope)
    }
}

pub(super) struct RuntimeV3SyntheticGuard<'a> {
    pub(super) provider_kind: Option<&'a str>,
    pub(super) live_episode: bool,
    pub(super) lifecycle_enabled: bool,
    pub(super) lookup_binding_enabled: bool,
}

pub(super) enum RuntimeV3Admission {
    Ordinary(ExoRuntimeAdmission),
    Synthetic(Box<SyntheticRuntimeAdmission>),
}

pub(super) enum RuntimeV3Transport<T: ExoTransport> {
    Ordinary(AdmittedExoRuntimeTransport<T>),
    Synthetic(Box<SyntheticExoAdmittedTransport>),
}

pub(super) type RuntimeV3AdmittedTransport = RuntimeV3Transport<ExoProcessTransport>;

impl<T: ExoTransport> ExoTransport for RuntimeV3Transport<T> {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        match self {
            Self::Ordinary(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
            Self::Synthetic(transport) => {
                transport.exchange(request, max_response_bytes, timeout_millis)
            }
        }
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        match self {
            Self::Ordinary(transport) => transport.close(),
            Self::Synthetic(transport) => transport.close(),
        }
    }
}

impl RuntimeV3Admission {
    pub(super) fn mode(&self) -> RuntimeV3AdmissionMode {
        match self {
            Self::Ordinary(admission) => match admission.mode() {
                ExoAdmissionMode::Enveloped => RuntimeV3AdmissionMode::Enveloped,
                ExoAdmissionMode::Legacy => RuntimeV3AdmissionMode::Legacy,
            },
            Self::Synthetic(_) => RuntimeV3AdmissionMode::SyntheticEnvelope,
        }
    }

    pub(super) fn admit(
        &self,
        process: ExoProcessConfig,
    ) -> Result<RuntimeV3AdmittedTransport, String> {
        match self {
            Self::Ordinary(admission) => admission
                .admit(ExoProcessTransport::new(process))
                .map(RuntimeV3AdmittedTransport::Ordinary)
                .map_err(String::from),
            Self::Synthetic(admission) => admission
                .admit_transport(process)
                .map(|transport| RuntimeV3AdmittedTransport::Synthetic(Box::new(transport))),
        }
    }
}

struct InspectedDeployment {
    identity: ExoIdentity,
    private_state: sts2_harness::exo_bridge_configuration::PrivateStateProfile,
}

fn inspected_deployment(
    process: &ExoProcessConfig,
    package_path: &str,
    native_instance_id: &str,
) -> Result<InspectedDeployment, String> {
    let [mode, path, digest] = process.arguments() else {
        return Err(
            "Exo envelope requires --run or --run-v2, absolute configuration path and digest"
                .to_owned(),
        );
    };
    if !matches!(mode.as_str(), "--run" | "--run-v2") || !std::path::Path::new(path).is_absolute() {
        return Err(
            "Exo envelope requires --run or --run-v2, absolute configuration path and digest"
                .to_owned(),
        );
    }
    let loaded = sts2_harness::exo_bridge_configuration::load(path)
        .map_err(|code| format!("Exo deployment inspection refused: {code}"))?;
    let package = std::fs::canonicalize(package_path)
        .map_err(|_| "Exo package locator is unavailable".to_owned())?;
    let executor = std::fs::canonicalize(&loaded.config.executor)
        .map_err(|_| "Exo configured executor is unavailable".to_owned())?;
    if &loaded.digest != digest || package != executor {
        return Err(
            "Exo deployment configuration or package locator does not match launch".to_owned(),
        );
    }
    let identity = loaded
        .inspected_identity(
            std::path::Path::new(process.executable()),
            native_instance_id,
        )
        .map_err(|code| format!("Exo deployment inspection refused: {code}"))?;
    Ok(InspectedDeployment {
        identity,
        private_state: loaded.private_state,
    })
}

/// Inspects the artifacts the launch can bind to an explicit locator. The bridge executable and the
/// operator-located package artifact (`STS2_EXO_PACKAGE_PATH`) are each read whole through the
/// bounded `ExoInspectedArtifacts::read` and hashed into the inspected identity, so that identity
/// reflects the bytes actually on disk rather than the operator's declaration. A locator that is
/// missing, empty or unreadable is an error rather than an admission, so the seam stays fail-closed.
/// Every axis without inspected bytes stays unbound, and the reviewed preflight still refuses it as
/// `UnboundIdentity(axis)`.
fn inspected_artifacts(
    bridge_executable: &str,
    package_path: &str,
) -> Result<ExoInspectedArtifacts, String> {
    let bridge = ExoInspectedArtifacts::read(bridge_executable).map_err(|error| {
        format!("Exo admission cannot inspect the bridge artifact {bridge_executable}: {error}")
    })?;
    let package = ExoInspectedArtifacts::read(package_path).map_err(|error| {
        format!("Exo admission cannot inspect the package artifact {package_path}: {error}")
    })?;
    Ok(ExoInspectedArtifacts {
        package: Some(package),
        bridge: Some(bridge),
        ..ExoInspectedArtifacts::default()
    })
}

/// Admits the runtime bridge for the assembled deployment. The runtime takes its transport only
/// here, so it cannot dispatch one that the reviewed preflight has not admitted.
pub(super) fn admit(
    admission: &RuntimeV3Admission,
    process: ExoProcessConfig,
) -> Result<RuntimeV3AdmittedTransport, String> {
    admission.admit(process)
}

fn private_state_root() -> Result<String, String> {
    Ok(optional("STS2_EXO_PRIVATE_STATE_ROOT")?
        .unwrap_or_else(|| DEFAULT_PRIVATE_STATE_ROOT.to_owned()))
}

#[cfg(test)]
#[path = "runtime_v3_admission_inspection_tests.rs"]
mod inspection_tests;
