// SPDX-License-Identifier: MIT

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

pub const MAX_IDENTIFIER_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeError {
    Empty,
    TooLong,
    InvalidCharacters,
    InvalidVersion,
    InvalidDigest,
    OutOfRange,
}

impl fmt::Display for TypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "value is empty",
            Self::TooLong => "value is too long",
            Self::InvalidCharacters => "value contains invalid characters",
            Self::InvalidVersion => "version is not major.minor.patch",
            Self::InvalidDigest => "digest is not lowercase sha-256",
            Self::OutOfRange => "integer exceeds the v1 bound",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TypeError {}

fn validate_identifier(value: &str) -> Result<(), TypeError> {
    if value.is_empty() {
        return Err(TypeError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(TypeError::TooLong);
    }
    if !value.is_ascii() {
        return Err(TypeError::InvalidCharacters);
    }
    if !value.as_bytes()[0].is_ascii_alphanumeric()
        || value
            .bytes()
            .skip(1)
            .any(|byte| !byte.is_ascii_alphanumeric() && !b"._:-".contains(&byte))
    {
        return Err(TypeError::InvalidCharacters);
    }
    Ok(())
}

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, TypeError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl TryFrom<String> for $name {
            type Error = TypeError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = TypeError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl FromStr for $name {
            type Err = TypeError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier!(WorkflowId);
identifier!(GraphId);
identifier!(NodeId);
identifier!(InvocationId);
identifier!(RegionId);
identifier!(PlanId);
identifier!(RunId);
identifier!(EpisodeId);
identifier!(InstanceId);
identifier!(SessionId);
identifier!(LeaseId);
identifier!(OperationId);
identifier!(ProviderExecutionId);
identifier!(CommandId);
identifier!(TraceId);
identifier!(RequestId);
identifier!(ActionId);
identifier!(CatalogId);
identifier!(PolicyId);
identifier!(CapabilityId);
identifier!(ArtifactId);
identifier!(PromptId);
identifier!(RegistryId);
identifier!(CompilerId);
identifier!(ProducerId);
identifier!(ActorId);
identifier!(ProfileId);
identifier!(ProjectionId);
identifier!(SelectorId);
identifier!(OperationRef);
identifier!(ContextId);
identifier!(DecisionProfileId);
identifier!(PlannerProfileId);
identifier!(ArtifactKindId);
identifier!(ReasonCode);
identifier!(LabelId);
identifier!(GuardId);
identifier!(OutputId);
identifier!(FieldId);
