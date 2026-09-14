// SPDX-License-Identifier: MIT

use super::checkout_pin_matches;

const REPOSITORY: &str = "AI-Ascension/ascension-context-console";
const REVISION: &str = "df36452adcfa1b1c3a7f968be243cd25a02433c3";
const ACTION: &str = "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1";

fn workflow(step: &str) -> String {
    format!(
        "name: test\non: [pull_request]\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n{step}\n"
    )
}

fn checkout() -> String {
    format!(
        "      - uses: {ACTION}\n        with:\n          repository: {REPOSITORY}\n          ref: {REVISION}"
    )
}

fn matches(source: &str) -> bool {
    checkout_pin_matches(source, REPOSITORY, REVISION)
}

#[test]
fn valid_checkout_accepts_yaml_mappings_without_textual_adjacency() {
    assert!(matches(&workflow(&checkout())));
    let quoted = format!(
        "      - with: {{ref: '{REVISION}', path: console, repository: \"{REPOSITORY}\"}}\n        uses: '{ACTION}'"
    );
    assert!(matches(&workflow(&quoted)));
}

#[test]
fn shell_heredocs_and_unrelated_actions_do_not_establish_a_checkout() {
    let heredoc = format!(
        "      - run: |\n          cat <<'EOF'\n          repository: {REPOSITORY}\n          ref: {REVISION}\n          EOF"
    );
    for source in [
        workflow(&heredoc),
        workflow(&checkout().replace(ACTION, "actions/upload-artifact@v4")),
        workflow(&checkout().replace(&format!("uses: {ACTION}"), "name: no action")),
        workflow(&checkout().replace(
            ACTION,
            "other/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
        )),
        workflow(&checkout().replace(ACTION, "actions/checkout@main")),
        workflow(&checkout().replace(REPOSITORY, "AI-Ascension/other")),
        workflow(&checkout().replace(REVISION, &format!("{REVISION}0"))),
        format!("repository: {REPOSITORY}\nref: {REVISION}\n"),
    ] {
        assert!(!matches(&source), "{source}");
    }
}

#[test]
fn conditional_and_nonexecuting_steps_or_jobs_do_not_establish_a_pin() {
    let source = workflow(&checkout());
    for invalid in [
        source.replace("        with:", "        if: false\n        with:"),
        source.replace("        with:", "        if: '${{ false }}'\n        with:"),
        source.replace("        with:", "        run: echo unused\n        with:"),
        source.replace(
            "        with:",
            "        continue-on-error: true\n        with:",
        ),
        source.replace("    steps:", "    if: false\n    steps:"),
        source.replace("    steps:", "    needs: missing\n    steps:"),
        source.replace("    steps:", "    strategy: {matrix: {os: []}}\n    steps:"),
        source.replace("    steps:", "    continue-on-error: true\n    steps:"),
        source.replace("    runs-on: ubuntu-latest\n", ""),
        source.replace("ubuntu-latest", "'${{ inputs.runner }}'"),
    ] {
        assert!(!matches(&invalid), "{invalid}");
    }
}

#[test]
fn ambiguous_or_malformed_yaml_fails_closed() {
    let source = workflow(&checkout());
    for invalid in [
        source.replace("        with:", "        with: {ref: wrong}\n        with:"),
        source.replace("          ref:", "          ref: wrong\n          ref:"),
        source.replace("  test:", "  test: {}\n  test:"),
        format!("{source}\n---\n{source}"),
        source.replace("        with:", "        with: &inputs"),
        source.replace("        with:", "        with: !!map"),
        source.replace("        with:", "        with: *inputs"),
        source.replace(
            "          ref:",
            "          <<: {ref: wrong}\n          ref:",
        ),
        source.replace("    steps:", "    steps: ["),
    ] {
        assert!(!matches(&invalid), "{invalid}");
    }
}

#[test]
fn parser_resources_are_bounded_before_tree_loading() {
    let source = workflow(&checkout());
    assert!(!matches(&format!("{}{}", "#".repeat(65_536), source)));
    let nested = format!("extra: {}0{}\n{source}", "[".repeat(33), "]".repeat(33));
    assert!(!matches(&nested));
    let numerous = format!("extra: [{}]\n{source}", vec!["0"; 8192].join(","));
    assert!(!matches(&numerous));
}

#[test]
fn actual_committed_workflows_have_the_reviewed_checkout() {
    assert!(matches(include_str!(
        "../../../../.github/workflows/console-contract.yml"
    )));
    assert!(checkout_pin_matches(
        include_str!("../../../../.github/workflows/studio-contract.yml"),
        "AI-Ascension/ascension-workflow-studio",
        "31c5e5f407ab17fb0363374dd1d926c25e6350ea",
    ));
}
