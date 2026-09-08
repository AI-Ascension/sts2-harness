// SPDX-License-Identifier: MIT

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use sts2_harness::{Decision, DecisionInput, DecisionSource, PolicyError};

use super::*;

#[path = "runtime_v3_recovery_fixture.rs"]
mod recovery_fixture;
pub(super) use recovery_fixture::{recovery_settled_script, response_script, settled_frames};

pub(super) const PENDING_OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";
pub(super) const PENDING_STATE_ID: &str = "22222222-2222-4222-8222-222222222222";
const RECOVERY_DEPLOYMENT_ID: &str = "33333333-3333-4333-8333-333333333333";
const RECOVERY_INSTANCE_ID: &str = "44444444-4444-4444-8444-444444444444";
const RECOVERY_INSTANCE_INCAR: &str = "55555555-5555-4555-8555-555555555555";
const RECOVERY_BOOT_ID: &str = "66666666-6666-4666-8666-666666666666";
const RECOVERY_LEASE_ID: &str = "77777777-7777-4777-8777-777777777777";
const RECOVERY_FENCE_ID: &str = "88888888-8888-4888-8888-888888888888";
const RECOVERY_PRINCIPAL_ID: &str = "99999999-9999-4999-8999-999999999999";
const RECOVERY_TICKET_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const RECOVERY_WITNESS_ID: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const RECOVERY_LOOKUP_MESSAGE_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const RECOVERY_RECONCILE_MESSAGE_ID: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const RECOVERY_LOOKUP_CORRELATION_ID: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const RECOVERY_RECONCILE_CORRELATION_ID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";

pub(super) struct RecoveryEnvironment(pub(super) Vec<(String, String)>);

pub(super) fn recovery_authority() -> super::allocation_context::RecoveryAuthority {
    super::allocation_context::RecoveryAuthority {
        deployment_id: RECOVERY_DEPLOYMENT_ID.to_owned(),
        instance_id: RECOVERY_INSTANCE_ID.to_owned(),
        instance_incarnation: RECOVERY_INSTANCE_INCAR.to_owned(),
        boot_id: RECOVERY_BOOT_ID.to_owned(),
        authority_generation: 1,
        lease_id: RECOVERY_LEASE_ID.to_owned(),
        lease_epoch: 1,
        current_fence: json!({
            "host_fence_id": RECOVERY_FENCE_ID,
            "deployment_id": RECOVERY_DEPLOYMENT_ID,
            "instance_id": RECOVERY_INSTANCE_ID,
            "instance_incarnation": RECOVERY_INSTANCE_INCAR,
            "boot_id": RECOVERY_BOOT_ID,
            "authority_generation": 1,
            "fence_generation": 1,
            "created_at": "2026-09-07T00:00:00Z"
        }),
    }
}

impl RecoveryEnvironment {
    pub(super) fn new() -> Self {
        let fence = json!({
            "host_fence_id": RECOVERY_FENCE_ID,
            "deployment_id": RECOVERY_DEPLOYMENT_ID,
            "instance_id": RECOVERY_INSTANCE_ID,
            "instance_incarnation": RECOVERY_INSTANCE_INCAR,
            "boot_id": RECOVERY_BOOT_ID,
            "authority_generation": 1,
            "fence_generation": 1,
            "created_at": "2026-09-07T00:00:00Z"
        });
        Self(
            vec![
                ("STS2_RECOVERY_TOKEN", "synthetic-recovery-token".to_owned()),
                (
                    "STS2_RECOVERY_PRINCIPAL_ID",
                    "synthetic-principal".to_owned(),
                ),
                ("STS2_RECOVERY_ROLE", "watchdog-recovery".to_owned()),
                ("STS2_RECOVERY_PROOF", "synthetic-proof".to_owned()),
                (
                    "STS2_RECOVERY_DEPLOYMENT_ID",
                    RECOVERY_DEPLOYMENT_ID.to_owned(),
                ),
                ("STS2_RECOVERY_INSTANCE_ID", RECOVERY_INSTANCE_ID.to_owned()),
                (
                    "STS2_RECOVERY_INSTANCE_INCAR",
                    RECOVERY_INSTANCE_INCAR.to_owned(),
                ),
                ("STS2_RECOVERY_BOOT_ID", RECOVERY_BOOT_ID.to_owned()),
                ("STS2_RECOVERY_LEASE_ID", RECOVERY_LEASE_ID.to_owned()),
                ("STS2_RECOVERY_AUTHORITY_GENERATION", String::from("1")),
                ("STS2_RECOVERY_LEASE_EPOCH", String::from("1")),
                ("STS2_RECOVERY_CURRENT_FENCE_JSON", fence.to_string()),
            ]
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
        )
    }
}

pub(super) struct Fixture(pub(super) PathBuf);

pub(super) struct CountingSource {
    pub(super) calls: usize,
    pub(super) decision: Decision,
}

impl DecisionSource for CountingSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        Ok(self.decision.clone())
    }
}

impl Fixture {
    pub(super) fn new() -> Result<Self, std::io::Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-v3-reconnect-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    pub(super) fn script(&self, content: &str) -> Result<String, Box<dyn std::error::Error>> {
        let path = self.0.join("mcp");
        fs::write(&path, format!("#!/bin/sh\n{content}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path.to_str().ok_or("non-UTF8 fixture path")?.to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.0);
    }
}

pub(super) fn reply(value: Value) -> String {
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
        value.to_string().replace('\'', "'\\''")
    )
}

pub(super) fn reply_text(text: &str) -> String {
    let response = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {"content": [{"text": text}]}
    });
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
        response.to_string().replace('\'', "'\\''")
    )
}

pub(super) fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[((first & 0x03) << 4 | second.unwrap_or(0) >> 4) as usize] as char);
        encoded.push(match second {
            Some(second) => {
                TABLE[((second & 0x0f) << 2 | third.unwrap_or(0) >> 6) as usize] as char
            }
            None => '=',
        });
        encoded.push(match third {
            Some(third) => TABLE[(third & 0x3f) as usize] as char,
            None => '=',
        });
    }
    encoded
}

pub(super) fn dispatch_script(fixture: &Fixture) -> Result<String, Box<dyn std::error::Error>> {
    let mut settled: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))?;
    settled["correlation_id"] = json!("1");
    settled["operation_id"] = json!("op-settled");
    let settled_text = settled
        .to_string()
        .replace("\"legal_actions\":[]", "\"legal_actions\" : [ ]");
    let tools: Vec<_> = [
        "sts2.observe",
        "sts2.legal_actions",
        "sts2.dispatch_action",
        "sts2.wait_for_transition",
        "sts2.reobserve",
        "sts2.recover",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let script = format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
        ),
        reply_text(&settled_text)
    );
    fixture.script(&script)
}

pub(super) fn synthetic_observation(
    state_id: &str,
    generation: u64,
    stage: &str,
    legal_actions: Value,
) -> Result<sts2_harness::EpisodeObservation, Box<dyn std::error::Error>> {
    let state = if stage == "victory" {
        json!({"state": stage})
    } else {
        json!({"state": stage, "turn_index": 1, "enemies": []})
    };
    let observation = json!({
        "state_id": state_id,
        "generation": generation,
        "visible_seed": "synthetic-seed",
        "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state": state,
        "legal_actions": legal_actions
    });
    Ok(sts2_harness::EpisodeObservation::new(
        state_id,
        generation,
        match stage {
            "combat" => sts2_harness::EpisodeStage::Combat,
            "victory" => sts2_harness::EpisodeStage::Victory,
            _ => return Err("unsupported synthetic stage".into()),
        },
        stage == "combat",
        stage != "combat",
        stage == "combat",
        observation,
    )?)
}
