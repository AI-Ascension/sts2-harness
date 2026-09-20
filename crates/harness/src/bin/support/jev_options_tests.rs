// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn transport_paths_preserve_spaces_without_changing_model_rules() -> Result<(), &'static str> {
    let transport = if cfg!(windows) {
        "C:/Program Files/System One/transport.exe"
    } else {
        "/opt/System One/transport"
    };
    for arguments in [
        vec!["--model", "jev-1.13.0", "--transport", transport],
        vec!["--transport", transport, "--model", "jev-1.13.0"],
    ] {
        let options = Options::parse(arguments.into_iter().map(str::to_owned))?;
        assert_eq!(options.transport.as_deref(), Some(transport));
        assert_eq!(options.model, "jev-1.13.0");
    }
    assert!(Options::parse(vec![String::from("--model"), String::from("jev latest")]).is_err());
    Ok(())
}

#[test]
fn option_like_path_text_is_not_parsed_as_flags() -> Result<(), &'static str> {
    let transport = if cfg!(windows) {
        "C:/System One/transport --gate 0 --record --describe --tactical.exe"
    } else {
        "/opt/System One/transport --gate 0 --record --describe --tactical"
    };
    let options = Options::parse(vec![String::from("--transport"), transport.to_owned()])?;
    assert_eq!(options.transport.as_deref(), Some(transport));
    assert_eq!(options.gate_percent, None);
    assert!(!options.record);
    assert!(!options.describe);
    assert!(!options.tactical);
    Ok(())
}

#[test]
fn transport_paths_reject_controls_and_non_absolute_values() {
    let transport = if cfg!(windows) {
        "C:/Program Files/System One/transport.exe"
    } else {
        "/opt/System One/transport"
    };
    for control in ['\0', '\n', '\r', '\t', '\u{7f}'] {
        let invalid = format!("{transport}{control}");
        assert!(Options::parse(vec![String::from("--transport"), invalid]).is_err());
    }
    for invalid in [
        "",
        "System One/transport",
        "C:System One/transport.exe",
        "--describe",
    ] {
        assert!(Options::parse(vec![String::from("--transport"), invalid.to_owned()]).is_err());
    }
}

#[test]
fn transport_path_limit_counts_bytes_and_keeps_the_inclusive_bound() -> Result<(), &'static str> {
    let prefix = if cfg!(windows) { "C:/" } else { "/" };
    let exact = format!("{prefix}{}", "x".repeat(MAX_TRANSPORT_BYTES - prefix.len()));
    let options = Options::parse(vec![String::from("--transport"), exact.clone()])?;
    assert_eq!(options.transport.as_deref(), Some(exact.as_str()));
    assert!(Options::parse(vec![String::from("--transport"), format!("{exact}x")]).is_err());
    let unicode = format!(
        "{prefix}{}é",
        "x".repeat(MAX_TRANSPORT_BYTES - prefix.len() - 1)
    );
    assert_eq!(unicode.chars().count(), MAX_TRANSPORT_BYTES);
    assert!(Options::parse(vec![String::from("--transport"), unicode]).is_err());
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
    assert_eq!(empty.transport, None);
    assert!(!empty.describe);
    assert!(!empty.record);

    let transport = if cfg!(windows) {
        "C:/providers/systemone-transport.exe"
    } else {
        "/opt/providers/systemone-transport"
    };
    for arguments in [
        vec!["--model", "jev-1.13.0", "--transport", transport],
        vec!["--transport", transport, "--model", "jev-1.13.0"],
    ] {
        let options = Options::parse(arguments.into_iter().map(str::to_owned))?;
        assert_eq!(options.model, "jev-1.13.0");
        assert_eq!(options.transport.as_deref(), Some(transport));
        assert!(!options.describe);
    }
    Ok(())
}

#[test]
fn malformed_ambiguous_or_relative_selections_are_rejected() {
    let transport = if cfg!(windows) {
        "C:/providers/systemone-transport.exe"
    } else {
        "/opt/providers/systemone-transport"
    };
    for arguments in [
        vec!["--model"],
        vec!["--model", ""],
        vec!["--model", "a b"],
        vec!["--model", "a\nb"],
        vec!["--model", "--transport"],
        vec!["--model", "a", "--model", "b"],
        vec!["--transport"],
        vec!["--transport", "relative/path"],
        vec!["--transport", ""],
        vec!["--transport", transport, "--transport", transport],
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
    let transport = if cfg!(windows) {
        "C:/providers/systemone-transport.exe"
    } else {
        "/opt/providers/systemone-transport"
    };
    let options = Options::parse(
        vec!["--record", "--transport", transport]
            .into_iter()
            .map(str::to_owned),
    )?;
    assert!(options.record);
    assert!(!options.describe);
    assert_eq!(options.transport.as_deref(), Some(transport));
    Ok(())
}
