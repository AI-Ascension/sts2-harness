// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

const ADR_DIRECTORY: &str = "docs/decisions";

pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut by_number: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in files {
        let relative = relative_text(root, path);
        if let Some(number) = adr_number(&relative) {
            by_number.entry(number).or_default().push(relative);
        }
    }
    let mut findings = Vec::new();
    for (number, mut records) in by_number {
        records.sort();
        let Some((first, duplicates)) = records.split_first() else {
            continue;
        };
        for duplicate in duplicates {
            findings.push(Finding::error(
                "ADR001",
                duplicate,
                format!(
                    "duplicate ADR number {number} also used by {first}; renumber one of the files"
                ),
            ));
        }
    }
    findings
}

fn adr_number(relative: &str) -> Option<String> {
    let name = relative.strip_prefix(ADR_DIRECTORY)?.strip_prefix('/')?;
    if name.contains('/') {
        return None;
    }
    let stem = name.strip_suffix(".md")?;
    let (number, _) = stem.split_once('-')?;
    (number.len() == 4 && number.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| number.to_owned())
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::findings;
    use crate::config::Policy;
    use crate::files::collect;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Result<Self, Box<dyn Error>> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "repo-policy-adr-{}-{stamp}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root)?;
            Ok(Self(root))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    fn write_decision(root: &std::path::Path, name: &str) -> Result<(), Box<dyn Error>> {
        let directory = root.join("docs/decisions");
        fs::create_dir_all(&directory)?;
        fs::write(directory.join(name), format!("# {name}\n"))?;
        Ok(())
    }

    fn policy() -> Result<Policy, Box<dyn Error>> {
        Ok(Policy::load(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../policy.toml"),
        )?)
    }

    fn repository_findings(root: &std::path::Path) -> Result<Vec<String>, Box<dyn Error>> {
        let files = collect(root, &policy()?)?;
        Ok(findings(root, &files)
            .iter()
            .map(crate::diagnostic::Finding::render)
            .collect())
    }

    #[test]
    fn fails_on_duplicate_adr_numbers() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        let root = &fixture.0;
        write_decision(root, "0007-alpha.md")?;
        write_decision(root, "0007-beta.md")?;
        write_decision(root, "0008-gamma.md")?;
        let files = collect(root, &policy()?)?;
        let findings = findings(root, &files);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "ADR001");
        assert_eq!(findings[0].path, "docs/decisions/0007-beta.md");
        assert!(findings[0].message.contains("docs/decisions/0007-alpha.md"));
        assert!(findings[0].message.contains("0007"));
        Ok(())
    }

    #[test]
    fn passes_on_unique_adr_numbers() -> Result<(), Box<dyn Error>> {
        let fixture = Fixture::new()?;
        let root = &fixture.0;
        write_decision(root, "0007-alpha.md")?;
        write_decision(root, "0008-gamma.md")?;
        write_decision(root, "ADR-WF-004-store.md")?;
        assert!(repository_findings(root)?.is_empty());
        Ok(())
    }

    #[test]
    fn real_repository_has_unique_adr_numbers() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let duplicates = repository_findings(&root)?;
        assert!(duplicates.is_empty(), "{}", duplicates.join("; "));
        Ok(())
    }
}
