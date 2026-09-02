// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeV2Error {
    Artifact(RuntimeV2ArtifactError),
    Decode,
    Encode,
    Invalid(&'static str),
    PostWriteDisconnect,
}

impl std::fmt::Display for RuntimeV2Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Artifact(error) => write!(formatter, "Runtime-v2 artifact error: {error}"),
            Self::Decode => formatter.write_str("Runtime-v2 JSON decoding failed"),
            Self::Encode => formatter.write_str("Runtime-v2 JSON encoding failed"),
            Self::Invalid(message) => write!(formatter, "invalid Runtime-v2 value: {message}"),
            Self::PostWriteDisconnect => {
                formatter.write_str("simulated post-write disconnect; outcome is unknown")
            }
        }
    }
}

impl std::error::Error for RuntimeV2Error {}
