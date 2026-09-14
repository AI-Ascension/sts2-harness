// SPDX-License-Identifier: MIT

use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::sha256_hex;

/// Reviewed model tools. Empty because terminal decisions travel in the assistant message and no
/// #127 read-only query adapters are admitted yet; a non-empty catalog is not admissible.
pub const REVIEWED_MODEL_TOOLS: [&str; 0] = [];

pub const EXO_RESTRICTED_DEFAULT_QUOTA_BYTES: u64 = 1 << 30;
pub const EXO_RESTRICTED_MAX_QUOTA_BYTES: u64 = 8 << 30;
pub const EXO_RESTRICTED_DEFAULT_RETENTION_DAYS: u32 = 7;
pub const EXO_RESTRICTED_MAX_RETENTION_DAYS: u32 = 30;
pub const EXO_RESTRICTED_PERMISSIONS_OCTAL: u16 = 0o700;

/// Any path component with one of these names is rejected regardless of position, so alternate
/// home spellings such as `/var/home/<user>` or `/mnt/home/<user>` cannot slip through.
const FORBIDDEN_ANY_COMPONENT: [&str; 3] = ["home", "root", "users"];
/// A first path component with one of these system names is rejected.
const FORBIDDEN_FIRST_COMPONENT: [&str; 9] = [
    "etc", "usr", "boot", "dev", "proc", "sys", "run", "media", "mnt",
];
const FORBIDDEN_GAME_MARKERS: [&str; 8] = [
    "slaythespire",
    "slaythespire2",
    "slay the spire 2",
    "steam",
    "steamapps",
    "steamlibrary",
    "steamuserdata",
    "saves",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoToolCatalog {
    pub tools: Vec<String>,
}

impl ExoToolCatalog {
    /// The reviewed catalog: an explicit, empty model-facing allowlist.
    #[must_use]
    pub const fn reviewed() -> Self {
        Self { tools: Vec::new() }
    }

    /// Fails closed on empty, non-identifier, duplicate, or unreviewed declared tool names.
    pub fn validate(&self) -> Result<(), ExoToolCatalogError> {
        for tool in &self.tools {
            if tool.is_empty() {
                return Err(ExoToolCatalogError::EmptyToolName);
            }
            if !is_plain_identifier(tool) {
                return Err(ExoToolCatalogError::InvalidToolName);
            }
        }
        if has_duplicates(&self.tools) {
            return Err(ExoToolCatalogError::DuplicateTool);
        }
        if self
            .tools
            .iter()
            .any(|tool| !REVIEWED_MODEL_TOOLS.contains(&tool.as_str()))
        {
            return Err(ExoToolCatalogError::UnknownTool);
        }
        Ok(())
    }

    #[must_use]
    pub fn catalog_digest(&self) -> String {
        let mut encoded = String::from("sts2.exo-tool-catalog-v1\u{1f}");
        encoded.push_str(&self.tools.len().to_string());
        encoded.push('\u{1f}');
        for tool in &self.tools {
            push_field(&mut encoded, tool);
        }
        sha256_hex(encoded)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoPrivateStatePolicy {
    pub state_root: String,
    pub cache_root: String,
    pub temp_root: String,
    pub quota_bytes: u64,
    pub max_retention_days: u32,
    pub permissions_octal: u16,
}

impl ExoPrivateStatePolicy {
    /// Fails closed on unsafe roots, overlapping roots, unbounded quota/retention, or loose modes.
    ///
    /// Validation is lexical only: it does not resolve symlinks or reparse points and does not touch
    /// the filesystem. Materialization must canonicalize each root and open with symlink-refusing
    /// semantics (`O_NOFOLLOW`/`openat2`) so a symlink target cannot escape the reviewed root. Path
    /// comparison is byte-exact and case-sensitive, so a case-insensitive filesystem may alias two
    /// roots that pass validation.
    pub fn validate(&self) -> Result<(), ExoPrivateStateError> {
        let roots = [
            (PrivateRootKind::State, Path::new(&self.state_root)),
            (PrivateRootKind::Cache, Path::new(&self.cache_root)),
            (PrivateRootKind::Temp, Path::new(&self.temp_root)),
        ];
        for (kind, path) in &roots {
            validate_root(*kind, path)?;
        }
        for left in 0..roots.len() {
            for right in (left + 1)..roots.len() {
                if roots[left].1 == roots[right].1 {
                    return Err(ExoPrivateStateError::DuplicateRoot);
                }
                if is_component_prefix(roots[left].1, roots[right].1)
                    || is_component_prefix(roots[right].1, roots[left].1)
                {
                    return Err(ExoPrivateStateError::NestedRoot);
                }
            }
        }
        if self.quota_bytes == 0 || self.quota_bytes > EXO_RESTRICTED_MAX_QUOTA_BYTES {
            return Err(ExoPrivateStateError::InvalidQuota);
        }
        if self.max_retention_days == 0
            || self.max_retention_days > EXO_RESTRICTED_MAX_RETENTION_DAYS
        {
            return Err(ExoPrivateStateError::InvalidRetention);
        }
        if self.permissions_octal != EXO_RESTRICTED_PERMISSIONS_OCTAL {
            return Err(ExoPrivateStateError::UnsafePermissions);
        }
        Ok(())
    }

    #[must_use]
    pub fn policy_digest(&self) -> String {
        let mut encoded = String::from("sts2.exo-private-state-v1\u{1f}");
        push_field(&mut encoded, &self.state_root);
        push_field(&mut encoded, &self.cache_root);
        push_field(&mut encoded, &self.temp_root);
        push_field(&mut encoded, &self.quota_bytes.to_string());
        push_field(&mut encoded, &self.max_retention_days.to_string());
        push_field(&mut encoded, &format!("{:o}", self.permissions_octal));
        sha256_hex(encoded)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExoRestrictedProfile {
    pub tool_catalog: ExoToolCatalog,
    pub state: ExoPrivateStatePolicy,
}

impl ExoRestrictedProfile {
    /// Builds a reviewed profile with an empty catalog under `root_base`; `validate` then enforces
    /// that the resulting private roots are absolute, non-home, and non-game.
    #[must_use]
    pub fn reviewed_private(root_base: impl AsRef<Path>) -> Self {
        let base = root_base.as_ref();
        Self {
            tool_catalog: ExoToolCatalog::reviewed(),
            state: ExoPrivateStatePolicy {
                state_root: base.join("state").to_string_lossy().into_owned(),
                cache_root: base.join("cache").to_string_lossy().into_owned(),
                temp_root: base.join("temp").to_string_lossy().into_owned(),
                quota_bytes: EXO_RESTRICTED_DEFAULT_QUOTA_BYTES,
                max_retention_days: EXO_RESTRICTED_DEFAULT_RETENTION_DAYS,
                permissions_octal: EXO_RESTRICTED_PERMISSIONS_OCTAL,
            },
        }
    }

    /// Validates the tool catalog and private-state policy together.
    pub fn validate(&self) -> Result<(), ExoRestrictedError> {
        self.tool_catalog
            .validate()
            .map_err(ExoRestrictedError::ToolCatalog)?;
        self.state
            .validate()
            .map_err(ExoRestrictedError::PrivateState)
    }

    #[must_use]
    pub fn profile_digest(&self) -> String {
        let mut encoded = String::from("sts2.exo-restricted-profile-v1\u{1f}");
        push_field(&mut encoded, &self.tool_catalog.catalog_digest());
        push_field(&mut encoded, &self.state.policy_digest());
        sha256_hex(encoded)
    }
}

#[path = "restricted_error.rs"]
mod error;

pub use error::{ExoPrivateStateError, ExoRestrictedError, ExoToolCatalogError, PrivateRootKind};

fn validate_root(kind: PrivateRootKind, path: &Path) -> Result<(), ExoPrivateStateError> {
    if !path.is_absolute() {
        return Err(ExoPrivateStateError::PathNotAbsolute(kind));
    }
    if path
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(ExoPrivateStateError::PathEscapes(kind));
    }
    if has_forbidden_prefix(path) || has_game_marker(path) {
        return Err(ExoPrivateStateError::ForbiddenPath(kind));
    }
    Ok(())
}

fn normal_components(path: &Path) -> impl Iterator<Item = &str> {
    path.components().filter_map(|component| match component {
        Component::Normal(name) => name.to_str(),
        _ => None,
    })
}

fn has_forbidden_prefix(path: &Path) -> bool {
    let components: Vec<&str> = normal_components(path).collect();
    let Some(first) = components.first() else {
        return true;
    };
    if FORBIDDEN_FIRST_COMPONENT
        .iter()
        .any(|name| first.eq_ignore_ascii_case(name))
    {
        return true;
    }
    components.iter().any(|component| {
        FORBIDDEN_ANY_COMPONENT
            .iter()
            .any(|name| component.eq_ignore_ascii_case(name))
    })
}

fn has_game_marker(path: &Path) -> bool {
    let components: Vec<String> = normal_components(path)
        .map(str::to_ascii_lowercase)
        .collect();
    FORBIDDEN_GAME_MARKERS.iter().any(|marker| {
        let marker = marker.to_ascii_lowercase();
        components
            .iter()
            .any(|component| component.contains(marker.as_str()))
    })
}

fn is_component_prefix(prefix: &Path, path: &Path) -> bool {
    let mut prefix = prefix.components();
    let mut path = path.components();
    loop {
        match (prefix.next(), path.next()) {
            (None, _) => return true,
            (Some(left), Some(right)) if left == right => {}
            _ => return false,
        }
    }
}

fn is_plain_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn has_duplicates(values: &[String]) -> bool {
    values
        .iter()
        .enumerate()
        .any(|(index, value)| values[index + 1..].contains(value))
}

fn push_field(encoded: &mut String, value: &str) {
    encoded.push_str(&value.len().to_string());
    encoded.push(':');
    encoded.push_str(value);
    encoded.push('\u{1f}');
}
