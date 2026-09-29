// SPDX-License-Identifier: MIT

use super::*;

/// The bridge owns the exchange now, so `--transport` is not a spelling of anything.
///
/// This is the operator-facing half of the two-digests-into-one change: a configured
/// `STS2_EXO_BRIDGE_ARGS_JSON` that still carries `--transport` must be refused loudly rather
/// than silently ignored, because accepting it would let a run believe a transport was pinned
/// when the bridge was in fact the only artifact.
#[test]
fn the_transport_option_is_refused_in_every_position() {
    let transport = if cfg!(windows) {
        "C:/Program Files/System One/transport.exe"
    } else {
        "/opt/System One/transport"
    };
    for arguments in [
        vec!["--transport", transport],
        vec!["--model", "jev-1.13.0", "--transport", transport],
        vec!["--transport", transport, "--model", "jev-1.13.0"],
        vec!["--transport", transport, "--transport", transport],
    ] {
        assert!(Options::parse(arguments.into_iter().map(str::to_owned)).is_err());
    }
    assert!(Options::parse(vec![String::from("--model"), String::from("jev latest")]).is_err());
}

#[test]
fn the_remaining_path_limit_counts_bytes_and_keeps_the_inclusive_bound() -> Result<(), &'static str>
{
    let prefix = if cfg!(windows) { "C:/" } else { "/" };
    let exact = format!("{prefix}{}", "x".repeat(MAX_DIRECTORY_BYTES - prefix.len()));
    let options = Options::parse(vec![String::from("--audit-dir"), exact.clone()])?;
    assert_eq!(options.audit_dir.as_deref(), Some(exact.as_str()));
    assert!(Options::parse(vec![String::from("--audit-dir"), format!("{exact}x")]).is_err());
    let unicode = format!(
        "{prefix}{}é",
        "x".repeat(MAX_DIRECTORY_BYTES - prefix.len() - 1)
    );
    assert_eq!(unicode.chars().count(), MAX_DIRECTORY_BYTES);
    assert!(Options::parse(vec![String::from("--audit-dir"), unicode]).is_err());
    Ok(())
}

#[test]
fn tactical_profile_is_explicit_and_cannot_be_duplicated() -> Result<(), &'static str> {
    assert!(!Options::parse(Vec::new())?.tactical);
    assert!(Options::parse(vec![String::from("--tactical")])?.tactical);
    assert!(Options::parse(vec![String::from("--tactical"); 2]).is_err());
    Ok(())
}

#[test]
fn selection_preserves_identifiers_and_the_default_model() -> Result<(), &'static str> {
    let empty = Options::parse(Vec::new())?;
    assert_eq!(empty.model, DEFAULT_MODEL);
    assert!(!empty.describe);
    assert!(!empty.record);

    for (model, arguments) in [
        ("jev-1.13.0", vec!["--model", "jev-1.13.0"]),
        ("jev-preview", vec!["--model", "jev-preview"]),
    ] {
        let options = Options::parse(arguments.into_iter().map(str::to_owned))?;
        assert_eq!(options.model, model);
        assert!(!options.describe);
    }
    Ok(())
}

#[test]
fn malformed_ambiguous_or_relative_selections_are_rejected() {
    for arguments in [
        vec!["--model"],
        vec!["--model", ""],
        vec!["--model", "a b"],
        vec!["--model", "a\nb"],
        vec!["--model", "--gate"],
        vec!["--model", "a", "--model", "b"],
        vec!["--audit-dir", "relative/path"],
        vec!["--audit-dir", ""],
        vec!["--describe", "--describe"],
        vec!["--record", "--record"],
        vec!["--unknown"],
    ] {
        assert!(
            Options::parse(arguments.clone().into_iter().map(str::to_owned)).is_err(),
            "expected {arguments:?} to be refused"
        );
    }
    assert!(Options::parse(vec!["--model".to_owned(), "x".repeat(241)]).is_err());
}

#[test]
fn a_confidence_gate_is_accepted_as_an_integer_percentage() -> Result<(), &'static str> {
    assert_eq!(Options::parse(Vec::new())?.gate_percent, None);
    let options = Options::parse(vec!["--gate", "35"].into_iter().map(str::to_owned))?;
    assert_eq!(options.gate_percent, Some(35));
    for arguments in [
        vec!["--gate"],
        vec!["--gate", ""],
        vec!["--gate", "101"],
        vec!["--gate", "0.35"],
        vec!["--gate", "-1"],
        vec!["--gate", "035"],
        vec!["--gate", "35", "--gate", "40"],
    ] {
        assert!(
            Options::parse(arguments.clone().into_iter().map(str::to_owned)).is_err(),
            "expected {arguments:?} to be refused"
        );
    }
    Ok(())
}

#[test]
fn describe_is_accepted_beside_a_selection() -> Result<(), &'static str> {
    let options = Options::parse(
        vec!["--describe", "--model", "jev-preview"]
            .into_iter()
            .map(str::to_owned),
    )?;
    assert!(options.describe);
    assert_eq!(options.model, "jev-preview");
    Ok(())
}

#[test]
fn a_record_is_requested_explicitly_and_defaults_off() -> Result<(), &'static str> {
    let options = Options::parse(vec!["--record"].into_iter().map(str::to_owned))?;
    assert!(options.record);
    assert!(!options.describe);
    Ok(())
}
