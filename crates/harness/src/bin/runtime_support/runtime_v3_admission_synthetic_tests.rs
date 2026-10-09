// SPDX-License-Identifier: MIT

use std::sync::{Arc, Mutex};

use super::synthetic::{validate_identity_pins, validate_synthetic_process};
use super::{
    RuntimeV3Admission, RuntimeV3AdmissionMode, RuntimeV3Transport, selected_mode,
    validate_synthetic_preconditions,
};
use sts2_harness::exo_admission::{AdmittedExoRuntimeTransport, ExoRuntimeAdmission};
use sts2_harness::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoIdentity};
use sts2_harness::{ExoProcessConfig, ExoTransport, ExoTransportError};

fn process(
    arguments: Vec<String>,
    working_directory: Option<String>,
    inherited_environment: Vec<String>,
) -> Result<ExoProcessConfig, String> {
    ExoProcessConfig::new(
        "/opt/h391-fixture/bridge",
        arguments,
        working_directory,
        inherited_environment,
    )
    .map_err(|error| error.to_string())
}

fn identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some("a".repeat(64)),
        extension_digest: Some("b".repeat(64)),
        bridge_digest: Some("c".repeat(64)),
        model_binding: Some("o3-pro".to_owned()),
        provider: Some("openai".to_owned()),
        endpoint: Some("http://127.0.0.1:4319".to_owned()),
        prompt_digest: Some("d".repeat(64)),
        tool_digest: Some("e".repeat(64)),
        config_digest: Some("f".repeat(64)),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some("instance-1".to_owned()),
    }
}

#[test]
fn selector_preserves_defaults_and_refuses_unknown_values() {
    assert_eq!(selected_mode(None), Ok(RuntimeV3AdmissionMode::Enveloped));
    assert_eq!(
        selected_mode(Some("envelope")),
        Ok(RuntimeV3AdmissionMode::Enveloped)
    );
    assert_eq!(
        selected_mode(Some("legacy")),
        Ok(RuntimeV3AdmissionMode::Legacy)
    );
    assert_eq!(
        selected_mode(Some("synthetic-envelope")),
        Ok(RuntimeV3AdmissionMode::SyntheticEnvelope)
    );
    for value in [
        "",
        "Envelope",
        "ENVELOPE",
        "legacy ",
        "admitted",
        "synthetic",
        "raw",
    ] {
        assert!(selected_mode(Some(value)).is_err(), "{value:?}");
    }
    assert_eq!(
        RuntimeV3Admission::Ordinary(ExoRuntimeAdmission::legacy()).mode(),
        RuntimeV3AdmissionMode::Legacy
    );
}

#[test]
fn synthetic_mode_fences_provider_lookup_lifecycle_and_live_before_inspection() {
    let mode = RuntimeV3AdmissionMode::SyntheticEnvelope;
    assert!(validate_synthetic_preconditions(mode, Some("exo"), false, false, false,).is_ok());
    for (provider, live, lifecycle, lookup) in [
        (Some("synthetic"), false, false, false),
        (Some("exo"), true, false, false),
        (Some("exo"), false, true, false),
        (Some("exo"), false, false, true),
        (None, false, false, false),
    ] {
        assert!(validate_synthetic_preconditions(mode, provider, live, lifecycle, lookup).is_err());
    }
}

#[test]
fn from_environment_refuses_synthetic_gates_before_process_inspection() -> Result<(), String> {
    let invalid_process = process(Vec::new(), None, Vec::new())?;
    for (provider, live, lifecycle, lookup, expected) in [
        (
            Some("other"),
            false,
            false,
            false,
            "requires STS2_PROVIDER_KIND=exo",
        ),
        (Some("exo"), true, false, false, "live episodes"),
        (Some("exo"), false, true, false, "lifecycle configuration"),
        (Some("exo"), false, false, true, "Harness lookup"),
    ] {
        let error = super::from_environment(
            RuntimeV3AdmissionMode::SyntheticEnvelope,
            &invalid_process,
            false,
            "instance-1",
            super::RuntimeV3SyntheticGuard {
                provider_kind: provider,
                live_episode: live,
                lifecycle_enabled: lifecycle,
                lookup_binding_enabled: lookup,
            },
        )
        .err()
        .ok_or("synthetic precondition should refuse before process inspection")?;
        assert!(error.contains(expected), "{error}");
    }
    Ok(())
}

#[test]
fn synthetic_process_requires_exact_arguments_and_empty_environment() -> Result<(), String> {
    let args = vec![
        "--synthetic".to_owned(),
        "/opt/h391-fixture/config.json".to_owned(),
        "f".repeat(64),
    ];
    let valid = process(args.clone(), None, Vec::new())?;
    let digest = "f".repeat(64);
    assert_eq!(
        validate_synthetic_process(&valid),
        Ok(("/opt/h391-fixture/config.json", digest.as_str()))
    );
    assert!(
        validate_synthetic_process(&process(
            vec!["--synthetic-v2".to_owned()],
            None,
            Vec::new(),
        )?)
        .is_err()
    );
    assert!(
        validate_synthetic_process(&process(args.clone(), None, vec!["PATH".to_owned()])?).is_err()
    );
    assert!(
        validate_synthetic_process(&process(args, Some("/tmp".to_owned()), Vec::new())?).is_err()
    );
    Ok(())
}

#[test]
fn identity_pins_reject_stale_fixed_axes_and_malformed_digests() {
    let base = identity();
    assert!(validate_identity_pins(&base, "instance-1").is_ok());
    let mut invalid = Vec::new();
    let mut changed = base.clone();
    changed.source_revision = "0".repeat(40);
    invalid.push(changed);
    let mut changed = base.clone();
    changed.contract_version = "other-contract".to_owned();
    invalid.push(changed);
    let mut changed = base.clone();
    changed.model_binding = Some("other-model".to_owned());
    invalid.push(changed);
    let mut changed = base.clone();
    changed.provider = Some("other-provider".to_owned());
    invalid.push(changed);
    let mut changed = base.clone();
    changed.native_instance_id = Some("stale-instance".to_owned());
    invalid.push(changed);
    for endpoint in [
        "http://localhost:4319",
        "http://192.0.2.1:4319",
        "http://127.0.0.1:0",
    ] {
        let mut changed = base.clone();
        changed.endpoint = Some(endpoint.to_owned());
        invalid.push(changed);
    }
    for slot in 0..6 {
        let mut changed = base.clone();
        match slot {
            0 => changed.package_digest = None,
            1 => changed.extension_digest = Some("bad".to_owned()),
            2 => changed.bridge_digest = None,
            3 => changed.prompt_digest = None,
            4 => changed.tool_digest = None,
            _ => changed.config_digest = None,
        }
        invalid.push(changed);
    }
    for identity in invalid {
        assert!(validate_identity_pins(&identity, "instance-1").is_err());
    }
}

struct TransportProbe(Arc<Mutex<(usize, usize, bool)>>);

impl ExoTransport for TransportProbe {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        let mut counts = self.0.lock().map_err(|_| ExoTransportError::Unavailable)?;
        counts.0 += 1;
        if counts.2 {
            return Err(ExoTransportError::Unavailable);
        }
        Ok(b"ok".to_vec())
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        let mut counts = self.0.lock().map_err(|_| ExoTransportError::Unavailable)?;
        counts.1 += 1;
        Ok(())
    }
}

#[test]
fn local_transport_sum_forwards_ordinary_exchange_and_close() -> Result<(), ExoTransportError> {
    let counts = Arc::new(Mutex::new((0, 0, false)));
    let ordinary = AdmittedExoRuntimeTransport::Legacy(TransportProbe(Arc::clone(&counts)));
    let mut transport = RuntimeV3Transport::Ordinary(ordinary);
    assert_eq!(transport.exchange(b"request", 64, 1000)?, b"ok");
    transport.close()?;
    counts.lock().map_err(|_| ExoTransportError::Unavailable)?.2 = true;
    assert!(matches!(
        transport.exchange(b"request", 64, 1000),
        Err(ExoTransportError::Unavailable)
    ));
    assert_eq!(
        *counts.lock().map_err(|_| ExoTransportError::Unavailable)?,
        (2, 1, true)
    );
    Ok(())
}
