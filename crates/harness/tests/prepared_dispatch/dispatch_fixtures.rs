// SPDX-License-Identifier: MIT

//! Shared synthetic fixtures for the prepared-dispatch conformance suites.
//!
//! Every fixture is application-controlled synthetic bytes.  No fixture is a real provider payload,
//! profile or game file, and no fixture starts a provider, a host or a game process.

use sts2_harness::context_capture::{
    CaptureComponent, CaptureComponentKind, CaptureMode, DispatchFences, MemoryCapture,
    PreparedApplicationInput, PreparedDispatchController,
};

pub fn capture() -> MemoryCapture {
    MemoryCapture::new(CaptureMode::Memory, 32, 1_048_576).expect("capture")
}

pub fn digest(label: &str) -> String {
    sts2_harness::sha256_hex(label.as_bytes())
}

/// The exact components each advertised exact adapter assembles, mirroring the bridge seams.
pub fn components(adapter_id: &str) -> Vec<CaptureComponent<'static>> {
    match adapter_id {
        "exo" => vec![
            CaptureComponent {
                kind: CaptureComponentKind::Stdin,
                ordinal: 0,
                media_type: "text/plain; charset=utf-8",
                bytes: b"stdin-fixture\n",
            },
            CaptureComponent {
                kind: CaptureComponentKind::OutputSchema,
                ordinal: 1,
                media_type: "application/schema+json",
                bytes: br#"{"type":"object","properties":{}}"#,
            },
            CaptureComponent {
                kind: CaptureComponentKind::Configuration,
                ordinal: 2,
                media_type: "application/json",
                bytes: br#"{"argv":["--fixture"],"cwd":"fixture"}"#,
            },
        ],
        _ => vec![CaptureComponent {
            kind: CaptureComponentKind::Opaque,
            ordinal: 0,
            media_type: "application/json",
            bytes: br#"{"model":"fixture","stream":false,"prompt":"fixture"}"#,
        }],
    }
}

pub fn approved(adapter_id: &str) -> PreparedApplicationInput {
    PreparedApplicationInput::prepare(
        adapter_id,
        "exec-fixture",
        Some("attempt-fixture"),
        &components(adapter_id),
    )
    .expect("prepared input")
}

pub fn base_fences(adapter_id: &str) -> DispatchFences {
    DispatchFences {
        adapter_id: adapter_id.to_owned(),
        model_id: "model-fixture".to_owned(),
        configuration_digest: digest("configuration-fixture"),
        state_digest: digest("state-fixture"),
        catalog_digest: digest("catalog-fixture"),
        profile_digest: digest("profile-fixture"),
        auth_digest: digest("auth-fixture"),
        history_digest: digest("history-fixture"),
        compaction_digest: digest("compaction-fixture"),
        policy_id: "policy-fixture".to_owned(),
        policy_version: 3,
        controller_epoch: 11,
        gate_epoch: 7,
        lease_epoch: 5,
        revocation_epoch: 2,
    }
}

/// Drafts and holds one dispatch, returning the fences it was approved against.
pub fn held(
    controller: &mut PreparedDispatchController,
    dispatch_id: &str,
    adapter_id: &str,
) -> DispatchFences {
    let fences = base_fences(adapter_id);
    controller
        .draft(dispatch_id, approved(adapter_id), fences.clone())
        .expect("draft");
    controller.commit(dispatch_id, &fences).expect("commit");
    fences
}
