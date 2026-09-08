// SPDX-License-Identifier: MIT

use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const LINUX: &[u8] =
    include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json");
const WINDOWS: &[u8] =
    include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/windows.json");

fn frame(json: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = BOOTSTRAP_MAGIC.to_vec();
    bytes.extend_from_slice(&u32::try_from(json.len())?.to_be_bytes());
    bytes.extend_from_slice(json);
    Ok(bytes)
}

#[test]
fn owner_fixtures_are_frozen_and_both_platforms_decode() -> Result<(), Box<dyn std::error::Error>> {
    for (bytes, digest) in [
        (
            LINUX,
            "d2b382ebb5c1525400c53486562ad27947aedd052608f59d81152f9dd106fcd8",
        ),
        (
            WINDOWS,
            "6034ef4ecc46d149b838d3a468cc7065546fb8f6450122675b41c81d748cab57",
        ),
    ] {
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), digest);
        let bootstrap = WorkerBootstrap::decode(&frame(bytes)?)?;
        assert_eq!(bootstrap.component_id(), "harness");
        assert_eq!(
            bootstrap.launch_nonce(),
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
        );
        assert_eq!(
            bootstrap.watchdog_boot_id(),
            "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
        );
    }
    let schema = include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/schema.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(schema)),
        "2a5321b37e5d2dd581d579cfbf7f6f00a37d13af624e21cbd409d9712a519d9d"
    );
    Ok(())
}

#[test]
fn framing_rejects_truncation_trailing_frames_magic_and_size()
-> Result<(), Box<dyn std::error::Error>> {
    let valid = frame(LINUX)?;
    for length in 0..valid.len() {
        assert!(WorkerBootstrap::decode(&valid[..length]).is_err());
    }
    let mut doubled = valid.clone();
    doubled.extend_from_slice(&valid);
    assert!(WorkerBootstrap::decode(&doubled).is_err());
    for length in [0_u32, 16_385, u32::MAX] {
        let mut changed = valid.clone();
        changed[8..12].copy_from_slice(&length.to_be_bytes());
        assert!(WorkerBootstrap::decode(&changed).is_err());
    }
    let mut changed = valid;
    changed[0] = b'X';
    assert!(WorkerBootstrap::decode(&changed).is_err());
    Ok(())
}

#[test]
fn both_objects_are_closed_and_every_field_is_required() -> Result<(), Box<dyn std::error::Error>> {
    for bytes in [LINUX, WINDOWS] {
        let base: Value = serde_json::from_slice(bytes)?;
        for object_path in [None, Some("expected_peer")] {
            let object = match object_path {
                None => &base,
                Some(path) => &base[path],
            }
            .as_object()
            .ok_or("fixture must be object")?;
            for field in object.keys() {
                let mut changed = base.clone();
                let target = match object_path {
                    None => &mut changed,
                    Some(path) => &mut changed[path],
                };
                target
                    .as_object_mut()
                    .ok_or("fixture must be object")?
                    .remove(field);
                assert!(
                    WorkerBootstrap::decode(&frame(&serde_json::to_vec(&changed)?)?).is_err(),
                    "{field}"
                );
            }
            let mut changed = base.clone();
            let target = match object_path {
                None => &mut changed,
                Some(path) => &mut changed[path],
            };
            target
                .as_object_mut()
                .ok_or("fixture must be object")?
                .insert("unknown".into(), json!(true));
            assert!(WorkerBootstrap::decode(&frame(&serde_json::to_vec(&changed)?)?).is_err());
        }
    }
    Ok(())
}

#[test]
fn raw_duplicate_and_noncanonical_numeric_tokens_fail() -> Result<(), Box<dyn std::error::Error>> {
    let text = std::str::from_utf8(LINUX)?;
    for changed in [
        text.replace("\"version\": 1", "\"version\": 1, \"version\": 1"),
        text.replace("\"pid\": 42", "\"pid\": 42, \"p\\u0069d\": 42"),
        text.replace(
            "\"platform\": \"linux\"",
            "\"platform\": \"linux\", \"platform\": \"linux\"",
        ),
        text.replace("\"version\": 1", "\"version\": 1.0"),
        text.replace("\"pid\": 42", "\"pid\": 42e0"),
        text.replace("\"uid\": 1000", "\"uid\": -0"),
    ] {
        assert!(WorkerBootstrap::decode(&frame(changed.as_bytes())?).is_err());
    }
    Ok(())
}

#[test]
fn semantic_bounds_and_platform_paths_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
    let linux: Value = serde_json::from_slice(LINUX)?;
    for (field, value) in [
        ("pid", json!(0)),
        ("pid", json!(4_294_967_296_u64)),
        ("creation_token", json!("0")),
        ("creation_token", json!("01")),
        ("creation_token", json!("18446744073709551616")),
        ("executable", json!("relative")),
        ("executable", json!("/\u{0}")),
        ("executable", json!(format!("/{}", "é".repeat(2048)))),
        ("executable_sha256", json!("A".repeat(64))),
        ("platform", json!("other")),
        ("gid", json!(4_294_967_296_u64)),
    ] {
        let mut changed = linux.clone();
        changed["expected_peer"][field] = value;
        assert!(
            WorkerBootstrap::decode(&frame(&serde_json::to_vec(&changed)?)?).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("component_id", "."),
        ("component_id", ".."),
        ("component_id", "bad/component"),
        ("launch_nonce", "aaaaaaaa-aaaa-4aaa-7aaa-aaaaaaaaaaaa"),
        ("watchdog_boot_id", "BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB"),
    ] {
        let mut changed = linux.clone();
        changed[field] = json!(value);
        assert!(WorkerBootstrap::decode(&frame(&serde_json::to_vec(&changed)?)?).is_err());
    }
    let windows: Value = serde_json::from_slice(WINDOWS)?;
    for sid in [
        "S-1-281474976710656-1",
        "S-1-5-4294967296",
        "S-1-05-18",
        "S-1-5",
    ] {
        let mut changed = windows.clone();
        changed["expected_peer"]["sid"] = json!(sid);
        assert!(WorkerBootstrap::decode(&frame(&serde_json::to_vec(&changed)?)?).is_err());
    }
    Ok(())
}
