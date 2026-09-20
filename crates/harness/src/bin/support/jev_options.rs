// SPDX-License-Identifier: MIT

//! Command-line configuration for the System One bridge, shared with runtime admission.
//!
//! The runtime re-parses the admitted argument vector with this parser before it launches the
//! bridge, exactly as it does for the Ollama bridge, so the admission gate and the executable agree
//! on what a valid invocation is rather than holding two opinions about it.
//!
//! Nothing here is inferred from game or model output, and the credential is not an argument: it
//! reaches the bridge through the operator-declared inherited environment, never a command line.

/// Model identifier used when the operator does not select one.
pub(super) const DEFAULT_MODEL: &str = "jev-latest";

/// Largest transport path this parser accepts.
const MAX_TRANSPORT_BYTES: usize = 4096;

/// Largest confidence gate this parser accepts, in hundredths.
///
/// The gate is taken as an integer percentage rather than a decimal so an argument vector carries
/// no locale-dependent separator and admission can compare it exactly.
const MAX_GATE_PERCENT: u32 = 100;

/// Command-line configuration, never inferred from game or model output.
pub(super) struct Options {
    /// Provider model identifier, sent unchanged.
    pub model: String,
    /// Absolute path of the operator-owned executable that performs the HTTPS exchange.
    pub transport: Option<String>,
    /// Print the requested configuration and exit without opening a connection.
    pub describe: bool,
    /// Print one record object instead of the bare decision: `schema`, `provider_call`,
    /// `provider_request`, `provider_response` and `decision`.
    ///
    /// A stored evidence file has to prove that the decision it publishes is a function of the
    /// response beside it. Writing the two fields down by hand cannot prove that, so the operator
    /// who records an exchange asks for the record the bridge itself assembled.
    ///
    /// The runtime shares this parser but rejects record mode: its stdout contract is one bare
    /// decision. Standalone invocations may request the fuller record for offline inspection.
    pub record: bool,
    /// Confidence at or above which an answer becomes an action, as an integer percentage.
    ///
    /// Absent means the bridge's own default applies. An operator lowers it when a lane's real
    /// confidence distribution sits below the default, which is a measurement rather than a taste.
    pub gate_percent: Option<u32>,
    /// Opt-in parallel evaluator. False preserves the legacy request and decision path.
    pub tactical: bool,
    /// Explicit private sidecar directory. Never enables extra provider calls or record stdout.
    pub audit_dir: Option<String>,
}

impl Options {
    /// Parses the argument vector, refusing unknown, duplicate, or malformed options.
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let mut arguments = arguments.into_iter();
        let mut model = None;
        let mut transport = None;
        let mut gate_percent = None;
        let mut describe = false;
        let mut record = false;
        let mut tactical = false;
        let mut audit_dir = None;
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--describe" if !describe => describe = true,
                "--record" if !record => record = true,
                "--tactical" if !tactical => tactical = true,
                "--audit-dir" if audit_dir.is_none() => {
                    let value = arguments.next().ok_or("missing audit directory")?;
                    if !valid_identifier(&value, MAX_TRANSPORT_BYTES)
                        || !std::path::Path::new(&value).is_absolute()
                    {
                        return Err("invalid audit directory");
                    }
                    audit_dir = Some(value);
                }
                "--model" if model.is_none() => {
                    let value = arguments.next().ok_or("missing model identifier")?;
                    if !valid_identifier(&value, 240) {
                        return Err("invalid model identifier");
                    }
                    model = Some(value);
                }
                "--transport" if transport.is_none() => {
                    let value = arguments.next().ok_or("missing transport path")?;
                    if !valid_transport(&value) {
                        return Err("invalid transport path");
                    }
                    transport = Some(value);
                }
                "--gate" if gate_percent.is_none() => {
                    let value = arguments.next().ok_or("missing confidence gate")?;
                    let parsed = value
                        .parse::<u32>()
                        .map_err(|_| "invalid confidence gate")?;
                    if parsed > MAX_GATE_PERCENT || value != parsed.to_string() {
                        return Err("invalid confidence gate");
                    }
                    gate_percent = Some(parsed);
                }
                _ => return Err("unknown or duplicate bridge option"),
            }
        }
        if record && audit_dir.is_some() {
            return Err("record stdout and redacted capture are mutually exclusive");
        }
        Ok(Self {
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
            transport,
            describe,
            record,
            gate_percent,
            tactical,
            audit_dir,
        })
    }
}

/// Whether a model identifier satisfies the provider selection contract.
///
/// Model identifiers remain whitespace-free. Executable paths have a separate validator because
/// spaces inside an already separated argument are literal path data, not additional options.
fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.starts_with('-')
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

/// Admits one bounded absolute executable path without trimming or splitting it.
///
/// The bridge passes this exact value to `Command::new`, never to a shell. Spaces therefore remain
/// path data. Control characters remain forbidden, including NUL and line separators used in logs.
fn valid_transport(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TRANSPORT_BYTES
        && !value.chars().any(char::is_control)
        && std::path::Path::new(value).is_absolute()
}

#[cfg(test)]
#[path = "jev_capture_options_tests.rs"]
mod capture_tests;

#[cfg(test)]
mod tests {
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
    fn transport_path_limit_counts_bytes_and_keeps_the_inclusive_bound() -> Result<(), &'static str>
    {
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
}
