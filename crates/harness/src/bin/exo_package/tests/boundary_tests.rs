// SPDX-License-Identifier: MIT

use super::*;
use std::os::unix::fs::symlink;

#[test]
fn admission_refuses_extra_modules_wrong_owned_bytes_and_unexecutable_inputs() -> TestResult {
    let fixture = Fixture::new()?;
    fixture.write_input("unreviewed.ts", b"export {};", false)?;
    assert!(manifest::build(&fixture.input, "0".repeat(40).as_str()).is_err());
    fs::remove_file(fixture.input.join("unreviewed.ts"))?;
    fixture.write_input("index.ts", b"export {};", false)?;
    assert!(manifest::build(&fixture.input, "0".repeat(40).as_str()).is_err());
    let extension = manifest::INPUTS
        .iter()
        .find(|input| input.name == "index.ts")
        .and_then(|input| input.owned)
        .ok_or("extension fixture")?;
    fixture.write_input("index.ts", extension, false)?;
    fs::set_permissions(
        fixture.input.join("node"),
        fs::Permissions::from_mode(0o600),
    )?;
    assert!(manifest::build(&fixture.input, "0".repeat(40).as_str()).is_err());
    Ok(())
}

#[test]
fn manifest_and_settings_reject_duplicate_unknown_and_unadmitted_axes() -> TestResult {
    let fixture = Fixture::new()?;
    let bytes = fs::read(&fixture.manifest)?;
    let mut value: serde_json::Value = serde_json::from_slice(&bytes)?;
    for (field, replacement) in [
        ("platform", "windows-x86_64"),
        ("exo_revision", "moving-main"),
        ("node", "22.15.0"),
        ("schema", "sts2.exo-install-manifest-v2"),
        ("unreviewed", "field"),
    ] {
        let mut negative = value.clone();
        negative[field] = serde_json::json!(replacement);
        assert!(manifest::decode(&manifest::encode(&negative)?).is_err());
    }
    value["components"][0]["name"] = serde_json::json!("../escape");
    assert!(manifest::decode(&manifest::encode(&value)?).is_err());
    let text = String::from_utf8(bytes)?;
    let duplicate = text.replacen('{', "{\"schema\":\"sts2.exo-install-manifest-v1\",", 1);
    assert!(manifest::decode(duplicate.as_bytes()).is_err());
    let settings_text = fs::read_to_string(&fixture.settings)?;
    let duplicate = settings_text.replacen('{', "{\"model\":\"o3-pro\",", 1);
    assert!(settings::decode(duplicate.as_bytes()).is_err());
    let mut settings: serde_json::Value = serde_json::from_str(&settings_text)?;
    settings["endpoint"] = serde_json::json!("https://api.openai.com/v1");
    assert!(
        settings::decode(&manifest::encode(&settings)?).is_err(),
        "synthetic cannot switch to live"
    );
    Ok(())
}

#[test]
fn symlinks_hardlinks_foreign_roots_and_loose_modes_fail_without_side_effects() -> TestResult {
    let fixture = Fixture::new()?;
    let original = fixture.input.join("node");
    fs::rename(&original, fixture.base.join("node-original"))?;
    symlink(fixture.base.join("node-original"), &original)?;
    assert!(fixture.install().is_err());
    fs::remove_file(&original)?;
    fs::hard_link(fixture.base.join("node-original"), &original)?;
    assert!(fixture.install().is_err());
    fs::remove_file(&original)?;
    fs::rename(fixture.base.join("node-original"), &original)?;
    fs::set_permissions(&original, fs::Permissions::from_mode(0o777))?;
    assert!(fixture.install().is_err());
    fs::set_permissions(&original, fs::Permissions::from_mode(0o700))?;
    fs::DirBuilder::new().mode(0o700).create(&fixture.root)?;
    fs::write(fixture.root.join("unrelated-agent"), b"do not modify")?;
    assert!(fixture.install().is_err());
    assert_eq!(
        fs::read(fixture.root.join("unrelated-agent"))?,
        b"do not modify"
    );
    assert_eq!(fs::read_dir(&fixture.root)?.count(), 1);
    Ok(())
}

#[test]
fn changed_input_and_wrong_review_digest_refuse_before_install_creation() -> TestResult {
    let fixture = Fixture::new()?;
    let result = install::install(
        &fixture.input,
        &fixture.manifest,
        &"0".repeat(64),
        &fixture.settings,
        &"0".repeat(64),
        &fixture.root,
    );
    assert!(result.is_err());
    assert!(!fixture.root.exists());
    fixture.write_input("sts2-exo-executor", b"#!/bin/sh\nexit 79\n", true)?;
    assert!(fixture.install().is_err());
    assert!(!fixture.root.exists());
    Ok(())
}

#[test]
fn second_installation_and_unsafe_private_entry_are_isolated() -> TestResult {
    let first = Fixture::new()?;
    let second = Fixture::new()?;
    let a = first.install()?;
    let b = second.install()?;
    let before = fs::read(second.root.join(&b).join("config.json"))?;
    let foreign = first.root.join(&a).join("private/cache/foreign");
    symlink(second.root.join(&b).join("private/state"), &foreign)?;
    assert!(inspection::verify(&first.root, &a).is_err());
    assert!(inspection::verify(&second.root, &b).is_ok());
    assert_eq!(fs::read(second.root.join(&b).join("config.json"))?, before);
    assert_eq!(
        fs::read_dir(second.root.join(&b).join("private/state"))?.count(),
        0
    );
    Ok(())
}

#[test]
fn busy_installer_cannot_be_treated_as_quiescent() -> TestResult {
    let fixture = Fixture::new()?;
    let id = fixture.install()?;
    let locked = install::root(&fixture.root, false)?;
    assert!(matches!(
        inspection::verify(&fixture.root, &id),
        Err("exo_package_install_busy")
    ));
    drop(locked);
    assert!(inspection::verify(&fixture.root, &id).is_ok());
    Ok(())
}
