// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use toml::Value;

const SUPPORTED_POLICY_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SizeCategory {
    RustProduction,
    RustTest,
    CsharpProduction,
    CsharpTest,
    Workflow,
    Markdown,
}

impl SizeCategory {
    fn key(self) -> &'static str {
        match self {
            Self::RustProduction => "rust_production",
            Self::RustTest => "rust_test",
            Self::CsharpProduction => "csharp_production",
            Self::CsharpTest => "csharp_test",
            Self::Workflow => "workflow",
            Self::Markdown => "markdown",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Budget {
    pub(crate) preferred: usize,
    pub(crate) maximum: usize,
}

#[derive(Debug)]
pub(crate) struct Policy {
    pub(crate) required_files: Vec<String>,
    pub(crate) required_preambles: BTreeMap<String, Vec<String>>,
    pub(crate) ignored_directories: BTreeSet<String>,
    pub(crate) ignored_path_prefixes: BTreeSet<String>,
    pub(crate) exemptions: BTreeMap<String, String>,
    limits: BTreeMap<&'static str, Budget>,
}

impl Policy {
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Self::parse(&text)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let value = toml::from_str::<Value>(text)
            .map_err(|error| format!("cannot parse policy.toml: {error}"))?;
        let root = value
            .as_table()
            .ok_or_else(|| "policy.toml root must be a table".to_owned())?;
        let version = root.get("policy_version").and_then(Value::as_integer);
        if version != Some(SUPPORTED_POLICY_VERSION) {
            return Err(format!(
                "policy_version must be {SUPPORTED_POLICY_VERSION}, found {version:?}"
            ));
        }

        let project = table(root.get("project"), "project")?;
        let required_preambles = match project.get("required_preambles") {
            Some(value) => preamble_table(
                value
                    .as_table()
                    .ok_or_else(|| "project.required_preambles must be a table".to_owned())?,
            )?,
            None => BTreeMap::new(),
        };
        let limits = table(root.get("limits"), "limits")?;
        let exemptions = table(root.get("exemptions"), "exemptions")?;
        Ok(Self {
            required_files: string_array(project.get("required_files"), "required_files")?,
            required_preambles,
            ignored_directories: string_array(
                project.get("ignored_directories"),
                "ignored_directories",
            )?
            .into_iter()
            .collect(),
            ignored_path_prefixes: string_array(
                project.get("ignored_path_prefixes"),
                "ignored_path_prefixes",
            )?
            .into_iter()
            .collect(),
            exemptions: string_table(exemptions, "exemptions")?,
            limits: parse_limits(limits)?,
        })
    }

    pub(crate) fn budget(&self, category: SizeCategory) -> Option<Budget> {
        self.limits.get(category.key()).copied()
    }

    /// A policy carrying only the preamble requirements under test.
    #[cfg(test)]
    pub(crate) fn with_preambles(required_preambles: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            required_files: Vec::new(),
            required_preambles,
            ignored_directories: BTreeSet::new(),
            ignored_path_prefixes: BTreeSet::new(),
            exemptions: BTreeMap::new(),
            limits: BTreeMap::new(),
        }
    }
}

/// Reads `project.required_preambles`, a map from exact repository-relative
/// path to the ordered lines that must appear in that file, in order, before
/// any other content.
fn preamble_table(
    table: &toml::map::Map<String, Value>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    table
        .iter()
        .map(|(path, value)| {
            let entry = value
                .as_table()
                .ok_or_else(|| format!("required_preambles[{path}] must be a table"))?;
            let markers = entry
                .get("markers")
                .ok_or_else(|| format!("required_preambles[{path}] must declare markers"))?;
            let markers = string_array(
                Some(markers),
                &format!("required_preambles[{path}].markers"),
            )?;
            if markers.is_empty() {
                return Err(format!(
                    "required_preambles[{path}].markers cannot be empty"
                ));
            }
            Ok((path.clone(), markers))
        })
        .collect()
}

fn parse_limits(
    table: &toml::map::Map<String, Value>,
) -> Result<BTreeMap<&'static str, Budget>, String> {
    let mut result = BTreeMap::new();
    for category in [
        SizeCategory::RustProduction,
        SizeCategory::RustTest,
        SizeCategory::CsharpProduction,
        SizeCategory::CsharpTest,
        SizeCategory::Workflow,
        SizeCategory::Markdown,
    ] {
        let key = category.key();
        let preferred = positive_usize(table.get(&format!("{key}_preferred")), key)?;
        let maximum = positive_usize(table.get(&format!("{key}_max")), key)?;
        if preferred > maximum {
            return Err(format!("{key}_preferred cannot exceed {key}_max"));
        }
        result.insert(key, Budget { preferred, maximum });
    }
    Ok(result)
}

fn table<'a>(
    value: Option<&'a Value>,
    key: &str,
) -> Result<&'a toml::map::Map<String, Value>, String> {
    value
        .and_then(Value::as_table)
        .ok_or_else(|| format!("{key} must be a table"))
}

fn string_array(value: Option<&Value>, key: &str) -> Result<Vec<String>, String> {
    let array = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{key} must be an array"))?;
    array
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn string_table(
    table: &toml::map::Map<String, Value>,
    key: &str,
) -> Result<BTreeMap<String, String>, String> {
    table
        .iter()
        .map(|(path, value)| {
            value
                .as_str()
                .map(|reason| (path.clone(), reason.to_owned()))
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect()
}

fn positive_usize(value: Option<&Value>, key: &str) -> Result<usize, String> {
    let integer = value
        .and_then(Value::as_integer)
        .ok_or_else(|| format!("{key} limit must be an integer"))?;
    usize::try_from(integer)
        .ok()
        .filter(|limit| *limit > 0)
        .ok_or_else(|| format!("{key} limit must be positive"))
}

#[cfg(test)]
mod tests {
    use super::{Policy, SizeCategory};

    const LIMITS: &str = r"
[limits]
rust_production_preferred = 10
rust_production_max = 20
rust_test_preferred = 11
rust_test_max = 21
csharp_production_preferred = 12
csharp_production_max = 22
csharp_test_preferred = 13
csharp_test_max = 23
workflow_preferred = 14
workflow_max = 24
markdown_preferred = 15
markdown_max = 25
";

    #[test]
    fn parses_complete_policy() -> Result<(), String> {
        let text = format!(
            "policy_version = 1\n[project]\nrequired_files = [\"README.md\"]\n\
             ignored_directories = [\"target\"]\nignored_path_prefixes = []\n[project.required_preambles]\n\
             {LIMITS}\n[exemptions]\n"
        );
        let policy = Policy::parse(&text)?;
        assert_eq!(policy.required_files, ["README.md"]);
        assert_eq!(
            policy
                .budget(SizeCategory::RustProduction)
                .map(|budget| budget.maximum),
            Some(20)
        );
        Ok(())
    }

    #[test]
    fn rejects_inverted_budget() {
        let text = format!(
            "policy_version = 1\n[project]\nrequired_files = []\n\
             ignored_directories = []\nignored_path_prefixes = []\n[project.required_preambles]\n\
             {}\n[exemptions]\n",
            LIMITS.replace("rust_production_max = 20", "rust_production_max = 5")
        );
        assert!(Policy::parse(&text).is_err());
    }
}
