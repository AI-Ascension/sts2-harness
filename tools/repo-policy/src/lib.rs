// SPDX-License-Identifier: MIT

mod adr;
mod config;
mod diagnostic;
mod files;
mod license;
mod markdown;
mod module_lexer;
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
    findings.extend(files::required_file_findings(root, &policy));
    findings.extend(files::exemption_findings(root, &policy));
    findings.extend(files::language_findings(root, &repository_files));
    findings.extend(size_findings);
    findings.extend(workflow::findings(root, &repository_files));
    findings.extend(license::findings(root, &repository_files));
    findings.extend(markdown::findings(root, &repository_files));
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
        diagnostics: findings.iter().map(Finding::render).collect(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::path::PathBuf;

    use super::{Finding, outcome};

    #[test]
    fn strict_mode_promotes_warnings() {
        let findings = [Finding::warning("SIZE001", "src/lib.rs", "large")];
        let result = outcome(1, true, &findings);
        assert_eq!(result.warnings, 1);
        assert_eq!(result.errors, 1);
        assert!(!result.passed(true));
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
