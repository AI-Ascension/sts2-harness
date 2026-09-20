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
#[path = "jev_options_tests.rs"]
mod tests;
