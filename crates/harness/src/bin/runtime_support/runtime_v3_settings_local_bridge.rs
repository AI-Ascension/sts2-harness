// SPDX-License-Identifier: MIT

//! Argument admission for the local provider-bridge lane.
//!
//! A local bridge is launched with an argument vector the operator declares, so admission re-parses
//! that vector with the same parser the bridge executable itself uses. Admission and the executable
//! therefore cannot hold two opinions about what a valid invocation is, and an option that only one
//! of them understands is refused rather than silently accepted by one side.
//!
//! Each kind admits one exact shape and nothing else. `--describe` is a human-facing option that
//! prints configuration without contacting anything, so it is never an execution argument.

#[path = "../support/ollama_options.rs"]
mod ollama_options;

#[path = "../support/jev_options.rs"]
mod jev_options;

/// Whether this argument vector is admitted for this provider kind.
///
/// An empty vector is admitted for every local kind: it selects each bridge's own defaults.
pub(super) fn arguments_allowed(provider: Option<&str>, arguments: &[String]) -> bool {
    if arguments.is_empty() {
        return true;
    }
    match provider {
        Some("ollama") => ollama_allowed(arguments),
        Some("typesafe-jev") => system_one_allowed(arguments),
        _ => false,
    }
}

/// Admits the exact two-element Ollama model selection.
fn ollama_allowed(arguments: &[String]) -> bool {
    if arguments.len() != 2 || arguments[0] != "--model" {
        return false;
    }
    ollama_options::Options::parse(arguments.iter().cloned())
        .is_ok_and(|options| !options.describe && options.model == arguments[1])
}

/// Admits the System One model and transport selection, with an optional confidence gate.
///
/// Two shapes only: the four-element model and transport pair, and that pair followed by
/// `--gate PERCENT`. The gate is admitted because a lane whose measured confidence sits below the
/// bridge default would otherwise never act, and because an operator changing it should be visible
/// in the recorded argument vector rather than hidden in a rebuild.
fn system_one_allowed(arguments: &[String]) -> bool {
    // Length first: a shorter vector must be refused, not indexed.
    let gated = match arguments.len() {
        4 => false,
        6 => true,
        _ => return false,
    };
    if arguments[0] != "--model" || arguments[2] != "--transport" {
        return false;
    }
    if gated && arguments[4] != "--gate" {
        return false;
    }
    jev_options::Options::parse(arguments.iter().cloned()).is_ok_and(|options| {
        !options.describe
            && options.model == arguments[1]
            && options.transport.as_deref() == Some(arguments[3].as_str())
            && options.gate_percent.map(|gate| gate.to_string()) == arguments.get(5).cloned()
    })
}

#[cfg(test)]
mod tests {
    use super::arguments_allowed;

    /// An absolute path that is valid on the platform running the test.
    fn transport() -> &'static str {
        if cfg!(windows) {
            "C:/providers/systemone.exe"
        } else {
            "/opt/providers/systemone"
        }
    }

    #[test]
    fn local_bridge_admission_allows_only_explicit_ollama_model_selection() {
        let selected = vec!["--model".to_owned(), "team/custom:7b".to_owned()];
        assert!(arguments_allowed(Some("ollama"), &selected));
        assert!(!arguments_allowed(Some("openai-astra"), &selected));
        assert!(!arguments_allowed(None, &selected));
        assert!(arguments_allowed(Some("ollama"), &[]));
        assert!(arguments_allowed(Some("openai-astra"), &[]));
        for arguments in [
            vec!["--describe"],
            vec!["--model", ""],
            vec!["--model", "--describe"],
            vec!["--model", "x", "--describe"],
            vec!["--endpoint", "example.invalid"],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(!arguments_allowed(Some("ollama"), &arguments));
        }
    }

    #[test]
    fn local_bridge_admission_allows_only_the_exact_system_one_selection() {
        let selected: Vec<String> = ["--model", "jev-1.13.0", "--transport", transport()]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert!(arguments_allowed(Some("typesafe-jev"), &selected));
        assert!(!arguments_allowed(Some("ollama"), &selected));
        assert!(!arguments_allowed(Some("openai-astra"), &selected));
        assert!(!arguments_allowed(None, &selected));
        assert!(arguments_allowed(Some("typesafe-jev"), &[]));

        let gated: Vec<String> = [
            "--model",
            "jev-1.13.0",
            "--transport",
            transport(),
            "--gate",
            "35",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert!(arguments_allowed(Some("typesafe-jev"), &gated));
        assert!(!arguments_allowed(Some("ollama"), &gated));
    }

    #[test]
    fn a_system_one_selection_outside_its_exact_shape_is_refused() {
        for arguments in [
            vec!["--model", "jev-1.13.0"],
            vec!["--transport", transport()],
            vec!["--transport", transport(), "--model", "jev-1.13.0"],
            vec!["--model", "jev-1.13.0", "--transport", "relative/path"],
            vec!["--model", "jev-1.13.0", "--transport", ""],
            vec!["--model", "", "--transport", transport()],
            vec!["--model", "jev-1.13.0", "--describe", transport()],
            vec!["--describe"],
            vec![
                "--model",
                "jev-1.13.0",
                "--transport",
                transport(),
                "--gate",
            ],
            vec![
                "--model",
                "jev-1.13.0",
                "--transport",
                transport(),
                "--gate",
                "101",
            ],
            vec![
                "--model",
                "jev-1.13.0",
                "--transport",
                transport(),
                "--seed",
                "35",
            ],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(
                !arguments_allowed(Some("typesafe-jev"), &arguments),
                "{arguments:?} must be refused"
            );
        }
    }
}
