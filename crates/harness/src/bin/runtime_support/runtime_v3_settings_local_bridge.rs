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
//! `--record` prints a record object that stands beside the decision rather than the decision
//! itself, and this lane reads the executable's stdout as the decision, so the record form is not
//! admitted here either.

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

/// Admits the existing model/transport pair and optional gate, with an opt-in --tactical suffix.
/// The recorded vector exposes the selected policy. Record/describe and arbitrary flags stay denied.
fn system_one_allowed(arguments: &[String]) -> bool {
    // Canonical capture suffix only. Do not widen record/describe or other provider kinds.
    let (arguments, audit_dir) = match arguments {
        [execution @ .., flag, directory] if flag == "--audit-dir" => {
            (execution, Some(directory.as_str()))
        }
        _ => (arguments, None),
    };
    if audit_dir.is_some() && !cfg!(unix) {
        return false;
    }
    // Only a final --tactical is admitted, with either existing execution shape.
    let tactical = arguments.last().is_some_and(|value| value == "--tactical");
    let execution = if tactical {
        &arguments[..arguments.len() - 1]
    } else {
        arguments
    };
    system_one_shape(execution, tactical, audit_dir)
}

fn system_one_shape(arguments: &[String], tactical: bool, audit_dir: Option<&str>) -> bool {
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
    // The record form is refused by this fixed shape rather than by a check of its own: appending
    // `--record` to the pair makes a five-element vector, which the length arm refuses, and no
    // admitted position can carry a flag because the parser rejects a value that begins with `-`.
    // `--describe` is excluded below for the longer-standing reason: this lane reads the
    // executable's stdout as the decision, not a configuration print.
    let mut parsed = arguments.to_vec();
    if tactical {
        parsed.push(String::from("--tactical"));
    }
    if let Some(directory) = audit_dir {
        parsed.extend([String::from("--audit-dir"), directory.to_owned()]);
    }
    jev_options::Options::parse(parsed).is_ok_and(|options| {
        !options.describe
            && !options.record
            && options.tactical == tactical
            && options.audit_dir.as_deref() == audit_dir
            && options.model == arguments[1]
            && options.transport.as_deref() == Some(arguments[3].as_str())
            && options.gate_percent.map(|gate| gate.to_string()) == arguments.get(5).cloned()
    })
}

#[cfg(test)]
#[path = "runtime_v3_capture_admission_tests.rs"]
mod capture_tests;

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

    /// The record form is refused because the admitted set is one fixed shape, not by a branch.
    ///
    /// The pair alone is admitted and the same pair with `--record` appended is not. If the length
    /// arm were ever widened to admit the five-element form, every remaining check would still
    /// pass, so this contrast is what holds the refusal in place.
    #[test]
    fn the_record_form_is_not_admitted_for_a_lane_that_reads_the_decision() {
        let admitted = vec![
            "--model".to_owned(),
            "jev-1.13.0".to_owned(),
            "--transport".to_owned(),
            transport().to_owned(),
        ];
        let mut recorded = admitted.clone();
        recorded.push("--record".to_owned());
        assert!(
            arguments_allowed(Some("typesafe-jev"), &admitted),
            "the model and transport pair is the admitted shape"
        );
        assert!(
            !arguments_allowed(Some("typesafe-jev"), &recorded),
            "{recorded:?} must be refused"
        );
        for arguments in [
            vec!["--record"],
            vec![
                "--model",
                "jev-1.13.0",
                "--transport",
                transport(),
                "--gate",
                "35",
                "--record",
            ],
        ] {
            let arguments = arguments.into_iter().map(str::to_owned).collect::<Vec<_>>();
            assert!(
                !arguments_allowed(Some("typesafe-jev"), &arguments),
                "{arguments:?} must be refused"
            );
        }
    }

    #[test]
    fn tactical_suffix_is_admitted_without_widening_other_options() {
        for gated in [false, true] {
            let mut args = vec!["--model", "jev-1.13.0", "--transport", transport()];
            if gated {
                args.extend(["--gate", "20"]);
            }
            args.push("--tactical");
            let mut args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert!(arguments_allowed(Some("typesafe-jev"), &args));
            assert!(!arguments_allowed(Some("ollama"), &args));
            args.push(String::from("--record"));
            assert!(!arguments_allowed(Some("typesafe-jev"), &args));
            let _ = args.pop();
            args.push(String::from("--tactical"));
            assert!(!arguments_allowed(Some("typesafe-jev"), &args));
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
