// SPDX-License-Identifier: MIT

use super::Options;

fn directory() -> &'static str {
    if cfg!(windows) {
        "C:/capture"
    } else {
        "/var/lib/jev-capture"
    }
}

#[test]
fn audit_directory_is_explicit_and_record_mode_is_exclusive() -> Result<(), &'static str> {
    assert!(Options::parse(Vec::new())?.audit_dir.is_none());
    let parsed = Options::parse(["--audit-dir", directory()].into_iter().map(str::to_owned))?;
    assert_eq!(parsed.audit_dir.as_deref(), Some(directory()));
    for args in [
        vec!["--audit-dir"],
        vec!["--audit-dir", "relative"],
        vec!["--audit-dir", ""],
        vec!["--audit-dir", directory(), "--audit-dir", directory()],
        vec!["--audit-dir", directory(), "--record"],
        vec!["--record", "--audit-dir", directory()],
        vec!["--audit-dir", "--describe"],
    ] {
        assert!(Options::parse(args.into_iter().map(str::to_owned)).is_err());
    }
    Ok(())
}

#[test]
fn audit_describe_remains_a_read_only_configuration_request() -> Result<(), &'static str> {
    let options = Options::parse(
        ["--describe", "--audit-dir", directory()]
            .into_iter()
            .map(str::to_owned),
    )?;
    assert!(options.describe);
    assert!(!options.record);
    Ok(())
}
