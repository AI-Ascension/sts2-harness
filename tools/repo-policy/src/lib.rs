// SPDX-License-Identifier: MIT

mod adr;
mod byte_scan;
mod config;
mod conflict;
mod diagnostic;
mod exemptions;
mod files;
mod license;
mod markdown;
mod module_lexer;
mod module_paths;
mod module_roots;
mod module_scan;
mod modules;
mod rust;
mod workflow;

use std::path::Path;

use config::Policy;
use diagnostic::{Finding, Severity};

#[derive(Debug)]
pub struct Outcome {
    pub checked_files: usize,
    pub warnings: usize,
    pub errors: usize,
    /// Breaches an exemption explicitly acknowledges. Reported, never failing.
    pub exempted: usize,
    pub diagnostics: Vec<String>,
}

impl Outcome {
    #[must_use]
    pub fn passed(&self, strict: bool) -> bool {
        self.errors == 0 && (!strict || self.warnings == 0)
    }
}

/// Checks repository policy under `root`.
///
/// # Errors
///
/// Returns an error when the root, policy configuration, or repository tree cannot be read.
pub fn check(root: &Path, strict: bool) -> Result<Outcome, String> {
    if !root.is_dir() {
        return Err(format!(
            "repository root is not a directory: {}",
            root.display()
        ));
    }
    let policy = Policy::load(&root.join("policy.toml"))?;
    let repository_files = files::collect(root, &policy)?;
    let (checked_files, size_findings) = files::size_findings(root, &repository_files, &policy);

    let mut findings = Vec::new();
    findings.extend(conflict::findings(root, &repository_files));
    findings.extend(files::required_file_findings(root, &policy));
    findings.extend(files::exemption_findings(root, &policy));
    findings.extend(files::language_findings(root, &repository_files));
    findings.extend(size_findings);
    findings.extend(workflow::findings(root, &repository_files));
    findings.extend(license::findings(root, &repository_files));
    findings.extend(markdown::findings(root, &repository_files, &policy));
    findings.extend(adr::findings(root, &repository_files));
    findings.extend(rust::findings(root));
    findings.extend(modules::findings(root, &repository_files));
    findings.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.rule.cmp(right.rule))
            .then_with(|| left.message.cmp(&right.message))
    });
    Ok(outcome(checked_files, strict, &findings))
}

fn outcome(checked_files: usize, strict: bool, findings: &[Finding]) -> Outcome {
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Warning)
        .count();
    let exempted = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Exempted)
        .count();
    let mut errors = findings
        .iter()
        .filter(|finding| finding.severity == Severity::Error)
        .count();
    if strict {
        errors += warnings;
    }
    Outcome {
        checked_files,
        warnings,
        errors,
        exempted,
        diagnostics: findings.iter().map(Finding::render).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use super::{Finding, outcome};
    use crate::Severity;

    #[test]
    fn strict_mode_promotes_warnings() {
        let findings = [Finding::warning("SIZE001", "src/lib.rs", "large")];
        let result = outcome(1, true, &findings);
        assert_eq!(result.warnings, 1);
        assert_eq!(result.errors, 1);
        assert!(!result.passed(true));
    }

    #[test]
    fn an_exempted_breach_is_reported_without_failing_the_gate() {
        // The exemption is the acknowledgement, so a waived hard-limit breach must be
        // visible in the output yet still let the tree converge under --strict.
        let findings = [Finding::exempted(
            "SIZE001",
            "src/lib.rs",
            "931 nonblank lines exceeds hard maximum 400; waived by a policy exemption",
        )];
        let result = outcome(1, true, &findings);
        assert_eq!(result.exempted, 1);
        assert_eq!(result.errors, 0);
        assert_eq!(result.warnings, 0);
        assert!(result.passed(true));
        assert!(result.diagnostics[0].starts_with("EXEMPTED SIZE001"));
    }

    #[test]
    fn an_exempted_breach_is_not_silently_dropped() {
        let findings = [Finding::exempted("SIZE001", "src/lib.rs", "waived")];
        let result = outcome(1, true, &findings);
        assert_eq!(result.diagnostics.len(), 1);
    }

    /// Pins that the real tree actually *reports* its waived breaches.
    ///
    /// `repository_satisfies_strict_policy` below only asserts that nothing fails, so it
    /// still passes if the `EXEMPTED` finding is computed and then discarded — which is
    /// the exact #569 defect, reintroduced with a green suite.
    ///
    /// The assertion is deliberately on the *property* — every over-limit file that has
    /// an exemption is reported as waived — rather than on a literal count. A fixed count
    /// would make this gate block the very fixes it exists to encourage: splitting an
    /// over-limit exempted file (#564) legitimately drops the count, and a hard-coded
    /// number turns each such improvement into a red build. The count is derived here
    /// from `policy.toml` and the real file sizes, so it moves with the tree.
    #[test]
    fn repository_reports_its_waived_breaches() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let outcome = super::check(&root, true)?;
        // Every file the policy exempts *and* that is genuinely over its hard limit must
        // appear as a reported `EXEMPTED` breach. Recomputing the expected set from the
        // same inputs `check` uses keeps the test honest without freezing a number.
        let policy = super::config::Policy::load(&root.join("policy.toml"))
            .map_err(|error| -> Box<dyn Error> { Box::from(error) })?;
        let repository_files = super::files::collect(&root, &policy)?;
        let (_checked, size_findings) =
            super::files::size_findings(&root, &repository_files, &policy);
        let expected_breaching_exemptions = size_findings
            .iter()
            .filter(|finding| finding.severity == Severity::Exempted)
            .count();
        assert_eq!(
            outcome.exempted,
            expected_breaching_exemptions,
            "every over-limit exempted file must be reported as a waived breach; \
             diagnostics were: {}",
            outcome.diagnostics.join("; ")
        );
        // When the repository has no over-limit exempted file left — the state reached once every
        // size exemption was replaced by a real split — there is correctly nothing to report, and
        // the only honest assertion is that none was invented. The counting assertion above is
        // what keeps the test meaningful in both states: it still fails if a waived breach is
        // dropped from the output, and it still passes at zero without special-casing.
        assert_eq!(
            outcome.exempted > 0,
            expected_breaching_exemptions > 0,
            "a waived breach must be reported exactly when the policy exempts an over-limit \
             file; diagnostics were: {}",
            outcome.diagnostics.join("; ")
        );
        Ok(())
    }

    /// The repository itself must satisfy strict policy. This is the check that a preferred-size
    /// regression actually breaks: `CHANGELOG.md` grew past `markdown_preferred` on `main`, and the
    /// policy workflow promotes that warning to an error, so every open pull request failed its
    /// policy gate. A unit fixture cannot catch that, because the budget is a property of the real
    /// tree. Keep this test so the repository cannot drift back over budget unnoticed.
    #[test]
    fn repository_satisfies_strict_policy() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let outcome = super::check(&root, true)?;
        assert!(
            outcome.passed(true),
            "repository fails strict policy: {}",
            outcome.diagnostics.join("; ")
        );
        Ok(())
    }
}
