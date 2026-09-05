// SPDX-License-Identifier: MIT

use super::{MAX_INPUT_BYTES, fingerprint, json_string, read_bounded};
use std::cell::Cell;
use std::io::{self, Read};

const MANIFEST: &str = r#"{"quarantine":{"status":"quarantined"}}"#;

#[test]
fn oversize_reader_consumes_only_one_byte_beyond_bound() {
    struct Endless<'a>(&'a Cell<usize>);
    impl Read for Endless<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            output.fill(b' ');
            self.0.set(self.0.get() + output.len());
            Ok(output.len())
        }
    }
    let consumed = Cell::new(0);
    assert!(read_bounded(Endless(&consumed)).is_err());
    assert_eq!(consumed.get(), MAX_INPUT_BYTES + 1);
}

#[test]
fn exact_bound_is_accepted_but_one_extra_byte_is_rejected() -> Result<(), String> {
    let mut bytes = MANIFEST.as_bytes().to_vec();
    bytes.resize(MAX_INPUT_BYTES, b' ');
    assert_eq!(
        read_bounded(bytes.as_slice())?.quarantine_status,
        "quarantined"
    );
    bytes.push(b' ');
    assert!(read_bounded(bytes.as_slice()).is_err());
    Ok(())
}

#[test]
fn quarantine_is_read_from_json_structure_not_a_substring() -> Result<(), String> {
    let input = br#"{"nested":{"quarantine":{"status":"promoted"}},"quarantine":{"reason":"status: eligible","status":"quarantined"}}"#;
    assert_eq!(read_bounded(&input[..])?.quarantine_status, "quarantined");
    let escaped = br#"{"quarantine":{"status":"quarant\u0069ned"}}"#;
    assert_eq!(read_bounded(&escaped[..])?.quarantine_status, "quarantined");
    Ok(())
}

#[test]
fn malformed_missing_ambiguous_and_invalid_statuses_are_rejected() {
    for input in [
        r#"not-json "quarantine": {"status":"promoted"}"#,
        r#"{"quarantine":{"status":"quarantined"}} trailing"#,
        r#"{"nested":{"quarantine":{"status":"promoted"}}}"#,
        r#"{"quarantine":null}"#,
        r#"{"quarantine":{"status":true}}"#,
        r#"{"quarantine":{"status":"unknown"}}"#,
        r#"{"quarantine":{"status":"quarantined","status":"promoted"}}"#,
        r#"{"quarantine":{"status":"quarantined"},"quarantine":{"status":"promoted"}}"#,
    ] {
        assert!(read_bounded(input.as_bytes()).is_err(), "accepted {input}");
    }
    assert!(read_bounded(&[0xff][..]).is_err());
}

#[test]
fn read_error_is_not_silently_treated_as_eof() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("fixture read failure"))
        }
    }
    assert!(read_bounded(Broken).is_err());
}

#[test]
fn fingerprint_and_path_encoding_preserve_input_bytes() -> Result<(), serde_json::Error> {
    assert_eq!(fingerprint(""), "fnv1a64-cbf29ce484222325");
    assert_eq!(fingerprint("a"), "fnv1a64-af63dc4c8601ec8c");
    let path = "control\u{0001}\n\"\\.json";
    assert_eq!(serde_json::from_str::<String>(&json_string(path))?, path);
    Ok(())
}
