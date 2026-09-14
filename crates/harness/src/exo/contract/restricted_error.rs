// SPDX-License-Identifier: MIT

//! Fail-closed error types for the restricted Exo profile contract.

/// Which declared private root failed validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivateRootKind {
    State,
    Cache,
    Temp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoToolCatalogError {
    EmptyToolName,
    InvalidToolName,
    DuplicateTool,
    UnknownTool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoPrivateStateError {
    PathNotAbsolute(PrivateRootKind),
    PathEscapes(PrivateRootKind),
    ForbiddenPath(PrivateRootKind),
    DuplicateRoot,
    NestedRoot,
    InvalidQuota,
    InvalidRetention,
    UnsafePermissions,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExoRestrictedError {
    ToolCatalog(ExoToolCatalogError),
    PrivateState(ExoPrivateStateError),
}

impl std::fmt::Display for ExoToolCatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyToolName => "declared Exo tool name is empty",
            Self::InvalidToolName => "declared Exo tool name is not a plain identifier",
            Self::DuplicateTool => "declared Exo tool catalog contains a duplicate",
            Self::UnknownTool => "declared Exo tool is not in the reviewed model allowlist",
        })
    }
}

impl std::error::Error for ExoToolCatalogError {}

impl std::fmt::Display for ExoPrivateStateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PathNotAbsolute(_) => "declared Exo private root is not absolute",
            Self::PathEscapes(_) => "declared Exo private root contains a parent component",
            Self::ForbiddenPath(_) => "declared Exo private root is forbidden",
            Self::DuplicateRoot => "declared Exo private roots are duplicated",
            Self::NestedRoot => "declared Exo private roots overlap",
            Self::InvalidQuota => "declared Exo private-state quota is outside the reviewed bound",
            Self::InvalidRetention => "declared Exo private-state retention is outside the bound",
            Self::UnsafePermissions => "declared Exo private-state permissions are not 0o700",
        })
    }
}

impl std::error::Error for ExoPrivateStateError {}

impl std::fmt::Display for ExoRestrictedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ToolCatalog(_) => "restricted Exo tool catalog is not admissible",
            Self::PrivateState(_) => "restricted Exo private-state policy is not admissible",
        })
    }
}

impl std::error::Error for ExoRestrictedError {}
