// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use super::super::seed_v2_support::{
    CountingAuthority, PrivateDirectory, client, derive_once_request, open_store, request_bytes,
    runtime_counts, send_abandoned_post, start_live_server,
};
use crate::management::SeededRunSubmissionResponseV2;

#[cfg(target_os = "linux")]
#[test]
fn served_seed_v2_replays_lost_responses_with_the_pinned_key_after_rotation_and_restart() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("workflow.sqlite");
    let key_v1 = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let key_rotated = directory.keyring(
        "keys-rotated.conf",
        "key-2",
        &[("key-1", "11"), ("key-2", "22")],
    );
    let key_without_old = directory.keyring("keys-new-only.conf", "key-2", &[("key-2", "22")]);
    let first_store = open_store(&database);
    let first_keys = Arc::new(CountingAuthority::open(&key_v1));
    let first_server = start_live_server(
        Arc::clone(&first_store),
        Arc::clone(&first_keys),
        "live.catalog.v1",
    );
    let (request, _) = derive_once_request();
    let first = client(&first_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("initial seed submission");
    assert_eq!(first.status, 200);
    let first: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&first.body).expect("initial response");
    assert_eq!(runtime_counts(&first_server.runtime_counters), (0, 0, 0, 0));

    let mut abandoned_request = request.clone();
    abandoned_request.request_id = "seed-lost-response".to_owned();
    abandoned_request.admission.as_mut().unwrap().request_id = abandoned_request.request_id.clone();
    let abandoned_body = request_bytes(&abandoned_request);
    let abandoned_response = send_abandoned_post(&first_server, &abandoned_body);
    let mut seed_count = 0_i64;
    for _ in 0..200 {
        seed_count = first_store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM management_seed_bindings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        if seed_count == 2 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    drop(abandoned_response);
    assert_eq!(
        seed_count, 2,
        "candidate commits before response abandonment"
    );
    first_server.server.shutdown().expect("stop first server");
    drop(first_store);

    let restarted_store = open_store(&database);
    let rotated_keys = Arc::new(CountingAuthority::open(&key_rotated));
    let restarted = start_live_server(
        Arc::clone(&restarted_store),
        Arc::clone(&rotated_keys),
        "live.catalog.rotated",
    );
    let replay = client(&restarted)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("replay after SQLite reopen");
    assert_eq!(replay.status, 200);
    let replay: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&replay.body).expect("replay response");
    assert_eq!(
        replay.seed_binding.effective_seed,
        first.seed_binding.effective_seed
    );
    assert_eq!(
        replay.seed_binding.operation_id,
        first.seed_binding.operation_id
    );
    assert_eq!(
        replay.seed_binding.configuration_digest,
        first.seed_binding.configuration_digest
    );
    assert_eq!(rotated_keys.current_reads(), 0);
    assert!(rotated_keys.pinned_reads() > 0);
    assert_eq!(
        restarted
            .catalog_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(runtime_counts(&restarted.runtime_counters), (0, 0, 0, 0));

    let abandoned_replay = client(&restarted)
        .request_json(
            "POST",
            "/v2/workflow-runs",
            Some(&request_bytes(&abandoned_request)),
        )
        .expect("abandoned-response replay");
    assert_eq!(abandoned_replay.status, 200);
    let abandoned_replay: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&abandoned_replay.body).expect("abandoned replay response");
    assert!(
        abandoned_replay.seed_binding.state
            == crate::management::SeedBindingStateV2::AwaitingHostContext
    );
    assert_eq!(rotated_keys.current_reads(), 0);

    let mut fresh_request = request.clone();
    fresh_request.request_id = "new-request-after-catalog-rotation".to_owned();
    fresh_request.admission.as_mut().unwrap().request_id = fresh_request.request_id.clone();
    let fresh = client(&restarted)
        .request_json(
            "POST",
            "/v2/workflow-runs",
            Some(&request_bytes(&fresh_request)),
        )
        .expect("new operation under rotated catalog");
    assert_eq!(fresh.status, 409);
    let fresh: serde_json::Value = serde_json::from_slice(&fresh.body).unwrap();
    assert_eq!(fresh["error"]["code"], "target_catalog_stale");
    assert_eq!(rotated_keys.current_reads(), 0);
    assert_eq!(
        restarted
            .catalog_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );

    let get = client(&restarted)
        .request_json(
            "GET",
            &format!(
                "/v2/workflow-runs/{}/seed-binding",
                replay.run.workflow_run_id
            ),
            None,
        )
        .expect("run-scoped readback");
    assert_eq!(get.status, 200);
    let readback: serde_json::Value = serde_json::from_slice(&get.body).unwrap();
    assert_eq!(
        readback["effective_seed"],
        replay.seed_binding.effective_seed
    );
    assert_eq!(readback["state"], "awaiting_host_context");

    restarted
        .server
        .shutdown()
        .expect("stop rotated-key server");
    drop(restarted_store);
    let missing_key_store = open_store(&database);
    let missing_keys = Arc::new(CountingAuthority::open(&key_without_old));
    let missing_key_server = start_live_server(
        Arc::clone(&missing_key_store),
        Arc::clone(&missing_keys),
        "live.catalog.rotated-again",
    );
    let missing_key = client(&missing_key_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("missing historical key refusal");
    assert_eq!(missing_key.status, 503);
    let missing_key: serde_json::Value = serde_json::from_slice(&missing_key.body).unwrap();
    assert_eq!(
        missing_key["error"]["code"],
        "seed_key_authority_unavailable"
    );
    assert_eq!(missing_keys.current_reads(), 0);
    assert_eq!(
        runtime_counts(&missing_key_server.runtime_counters),
        (0, 0, 0, 0)
    );
    missing_key_server
        .server
        .shutdown()
        .expect("stop missing-key server");
    drop(missing_key_store);
    directory.cleanup();
}
