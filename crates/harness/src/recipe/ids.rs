// SPDX-License-Identifier: MIT

//! Bounded identifiers and revisions for an authored recipe.

use std::fmt::{Display, Formatter};

/// Maximum accepted length, in bytes, of any recipe identifier component.
pub const MAX_IDENTIFIER_LEN: usize = 96;

/// Whether `value` is a non-empty, bounded, portable identifier.
///
/// Admission accepts only this shape so a recipe cannot smuggle a path, URL or
/// control characters into a tool mapping.
#[must_use]
pub fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
}

macro_rules! identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(String);

        impl $name {
            /// Admit a bounded identifier, or `None` when the shape is refused.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Option<Self> {
                let value = value.into();
                is_identifier(&value).then_some(Self(value))
            }

            /// Borrow the canonical identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(RecipeId, "Identifier of an authored pre-agent recipe.");
identifier!(
    StepId,
    "Identifier of one recipe step, unique within a recipe."
);
identifier!(ToolId, "Identifier of an approved read-only tool.");

macro_rules! revision {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(u32);

        impl $name {
            /// Wrap a revision number; zero is refused at admission.
            #[must_use]
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            /// The wrapped revision number.
            #[must_use]
            pub const fn get(self) -> u32 {
                self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                Display::fmt(&self.0, formatter)
            }
        }
    };
}

revision!(RecipeRevision, "Version of an authored recipe definition.");
revision!(ToolRevision, "Exact approved revision of a tool.");
