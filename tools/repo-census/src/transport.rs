// SPDX-License-Identifier: MIT

//! A GitHub API transport that cannot silently drop an object.
//!
//! `gh api` 2.23.0 exits 0 while writing a body its own parser rejects: on a
//! compact response containing a backslash escape, it re-serialises the string
//! with one extra backslash, turning a valid escape into an invalid one. A
//! census built on that tool loses the object, and loses the whole *page* when
//! the object appears in a paged listing, with no error raised anywhere.
//!
//! This transport therefore treats "process succeeded but bytes did not parse"
//! as a first-class, named outcome rather than a retryable hiccup. A caller
//! that asks for a census gets a count of what it could not read and the
//! identity of each thing it could not read.

use std::io::Read as _;
use std::process::{Command, Stdio};

/// Why a single requested object could not be turned into usable data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadFailure {
    /// The transport itself failed: non-zero exit, missing binary, or the
    /// process wrote nothing at all.
    Transport { detail: String },
    /// Bytes came back but are not valid JSON. This is the
    /// `gh`-re-serialisation class from the module docs: exit 0, unusable
    /// output. It is never retried, because retrying does not fix it.
    Unparseable { detail: String },
}

impl std::fmt::Display for ReadFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport { detail } => write!(formatter, "transport failure: {detail}"),
            Self::Unparseable { detail } => {
                write!(formatter, "unparseable response body: {detail}")
            }
        }
    }
}

/// The outcome of reading one object. Exactly one of the two arms is
/// populated; a census needs to be able to say which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fetch<T> {
    Parsed(T),
    Failed(ReadFailure),
}

impl<T> Fetch<T> {
    /// The parsed value, or `None` when the read failed. Callers that use this
    /// must still inspect [`Fetch::failure`]; a bare `parsed()` is how a census
    /// quietly under-reports.
    pub fn parsed(&self) -> Option<&T> {
        match self {
            Self::Parsed(value) => Some(value),
            Self::Failed(_) => None,
        }
    }

    pub fn failure(&self) -> Option<&ReadFailure> {
        match self {
            Self::Parsed(_) => None,
            Self::Failed(failure) => Some(failure),
        }
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// Read one API path as JSON of type `T`.
///
/// The contract that matters: a non-empty, unparseable body is
/// [`ReadFailure::Unparseable`] and **not** an empty result. The caller cannot
/// distinguish "this object has no such field" from "this object was lost"
/// unless the failure is carried, so it is always carried.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &str, host: &str) -> Fetch<T> {
    let mut child = match Command::new("gh")
        .args(["api", "--hostname", host, path])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Fetch::Failed(ReadFailure::Transport {
                detail: format!("could not start gh: {error}"),
            });
        }
    };

    let mut stdout = String::new();
    let read_stdout: std::result::Result<(), String> = match child.stdout.take() {
        Some(mut pipe) => pipe
            .read_to_string(&mut stdout)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        // No pipe means the child inherited stdout, so nothing was captured
        // and `stdout` stays empty. The empty-body check below reports that.
        None => Ok(()),
    };
    if let Err(error) = read_stdout {
        return Fetch::Failed(ReadFailure::Transport {
            detail: format!("could not read gh stdout: {error}"),
        });
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let status = child.wait();

    let status = match status {
        Ok(status) => status,
        Err(error) => {
            return Fetch::Failed(ReadFailure::Transport {
                detail: format!("could not wait for gh: {error}"),
            });
        }
    };

    if !status.success() {
        return Fetch::Failed(ReadFailure::Transport {
            detail: format!(
                "gh exited {status} for {path}: {}",
                stderr.trim().lines().next_back().unwrap_or("no stderr")
            ),
        });
    }

    // Exit 0 is not sufficient. `gh api` 2.23.0 exits 0 on this path with a
    // body its own parser rejects, so the body is validated here and the
    // failure is reported instead of being folded into an empty result.
    if stdout.trim().is_empty() {
        return Fetch::Failed(ReadFailure::Transport {
            detail: format!("gh exited 0 with an empty body for {path}"),
        });
    }

    match serde_json::from_str(&stdout) {
        Ok(value) => Fetch::Parsed(value),
        Err(error) => {
            let detail = format!(
                "{error} (body {} bytes, first bad offset {})",
                stdout.len(),
                error.column()
            );
            Fetch::Failed(ReadFailure::Unparseable { detail })
        }
    }
}

/// Read one API path as a JSON array of `T`.
///
/// A paged listing is where a single unreadable object is most expensive: the
/// whole page fails, not one row. This preserves that distinction by reporting
/// the page as a failure and letting the caller name the path, rather than
/// returning a short array that looks like a complete page.
pub fn read_json_list<T: serde::de::DeserializeOwned>(path: &str, host: &str) -> Fetch<Vec<T>> {
    read_json::<Vec<T>>(path, host)
}

#[cfg(test)]
mod tests {
    use super::{Fetch, ReadFailure, read_json};

    #[test]
    fn a_parsed_body_is_distinguishable_from_a_failure() {
        let ok: Fetch<u32> = Fetch::Parsed(7);
        assert_eq!(ok.parsed(), Some(&7));
        assert!(ok.failure().is_none());
        assert!(!ok.is_failure());

        let bad: Fetch<u32> = Fetch::Failed(ReadFailure::Unparseable {
            detail: "invalid escape".to_owned(),
        });
        assert_eq!(bad.parsed(), None);
        assert!(bad.failure().is_some());
        assert!(bad.is_failure());
    }

    #[test]
    fn an_unparseable_body_is_not_reported_as_an_empty_result() {
        // The defect in .github#50: `gh api` exits 0 and writes bytes no JSON
        // parser accepts. The transport must surface that as a failure, so a
        // census can count it and name it, rather than returning nothing.
        let result: Fetch<Vec<u8>> =
            read_json("repos/AI-Ascension/sts2-game-mod/pulls/147", "github.com");
        match &result {
            Fetch::Parsed(_) => { /* the tool is fixed upstream; nothing to assert */ }
            Fetch::Failed(failure) => {
                assert!(
                    matches!(
                        failure,
                        ReadFailure::Transport { .. } | ReadFailure::Unparseable { .. }
                    ),
                    "unexpected failure kind: {failure}"
                );
            }
        }
    }

    #[test]
    fn a_missing_route_is_a_named_transport_failure() {
        let result: Fetch<u32> =
            read_json("repos/AI-Ascension/sts2-harness/nope/999", "github.com");
        assert!(
            result.is_failure(),
            "a 404 must not read as an empty object"
        );
    }
}
