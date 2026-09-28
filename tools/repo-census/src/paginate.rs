// SPDX-License-Identifier: MIT

//! Reading a paged listing in full, and failing closed when a page cannot be read.
//!
//! # Why a single page is not a listing
//!
//! GitHub returns a `Link` header naming the next page. A caller that requests
//! page 1 and stops reports a smaller number than exists, with no error anywhere
//! -- and the tool this module belongs to would still exit zero having claimed a
//! clean measurement. Measured against `AI-Ascension/sts2-harness`, whose merged
//! pull requests run to six pages, that mistake reported 62 merged pull requests
//! where there are 452: 390 of them, 86%, silently lost.
//!
//! The only correct end to a traversal is the one the server names, so this
//! follows `rel="next"` until the server stops offering a successor. It never
//! computes a page count or stops on a short page, because both go stale the
//! moment a repository grows past an assumed size.
//!
//! # Failing closed
//!
//! A page that cannot be read ends the traversal and is reported as a failure
//! naming that exact page, alongside the pages that were read. The rows read
//! before a failure are a **prefix** of the listing, not the whole of it, and
//! the page count travels with them so a caller can say how much of the listing
//! it actually saw.

use std::io::Read as _;
use std::process::{Command, Stdio};

use crate::transport::ReadFailure;

/// One page of a paged listing, plus whatever the server said came next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page<T> {
    /// The parsed rows of this page, in server order.
    pub items: Vec<T>,
    /// The absolute URL of the next page, or `None` when this is the last.
    ///
    /// This comes from the `rel="next"` Link header rather than being computed
    /// from a row count, so the traversal follows the server's own idea of what
    /// comes next instead of a guess that stops early once a repository
    /// outgrows an assumed number of pages.
    pub next: Option<String>,
}

/// The result of a full traversal of a paged listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageOutcome<T> {
    /// Every page the server offered was read.
    Complete {
        /// Every row across every page, in server order.
        items: Vec<T>,
        /// How many pages that took. Reported so a caller can show the size of
        /// the traversal it actually performed, rather than leaving a reader to
        /// assume a single page was enough.
        pages: u64,
    },
    /// Traversal stopped at `failed_page`; `items` holds the pages read before
    /// it, which are a **prefix** of the listing and not the whole of it.
    Failed {
        items: Vec<T>,
        /// How many pages were read successfully before the failure.
        pages: u64,
        failed_page: String,
        failure: ReadFailure,
    },
}

impl<T> PageOutcome<T> {
    /// The rows read before any failure. A caller that looks only at this can
    /// still under-report, which is why [`PageOutcome::is_complete`] exists and
    /// why the census refuses to present an incomplete traversal as a
    /// measurement.
    pub fn items(&self) -> &[T] {
        match self {
            Self::Complete { items, .. } => items,
            Self::Failed { items, .. } => items,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete { .. })
    }

    /// How many pages the traversal read.
    pub fn pages(&self) -> u64 {
        match self {
            Self::Complete { pages, .. } | Self::Failed { pages, .. } => *pages,
        }
    }
}

/// Whether a Link header parameter list declares `rel="next"`.
fn declares_next(params: &str) -> bool {
    params
        .split(';')
        .map(str::trim)
        .any(|param| param == "rel=\"next\"")
}

/// Parse a `Link` header and return the URL of its `rel="next"` entry.
///
/// The header is scanned for `<...>` pairs, and the parameters belonging to a
/// URL are taken as the text from its closing `>` up to the next entry — the
/// next `<`, or the next `,` that is not inside a URL, whichever comes first.
/// Taking only up to the next `<` would fold the following URL into this one's
/// parameters, and a `;` inside that absorbed URL would then hide a real
/// `rel="next"`. A `next` entry that is not well formed yields `None`, which
/// ends the traversal at this page rather than guessing a successor.
pub fn next_link(header: Option<&str>) -> Option<String> {
    let header = header?;
    let mut cursor = 0;

    while let Some(offset) = header[cursor..].find('<') {
        let open = cursor + offset;
        let close = header[open + 1..].find('>').map(|index| open + 1 + index)?;
        let after_url = close + 1;
        let params_end = header[after_url..]
            .find([',', '<'])
            .map_or(header.len(), |index| after_url + index);

        if declares_next(&header[after_url..params_end]) {
            return Some(header[open + 1..close].to_owned());
        }
        cursor = params_end.max(close + 1);
    }

    None
}

/// Turn an absolute `next` URL from a Link header back into an `gh api` path.
///
/// The header points at `api.github.com`, while this tool reaches the API
/// through `gh api <path>`. The scheme, host, and API prefix are stripped and
/// what remains is the path `gh` is asked for. A URL that is not a GitHub API
/// URL is returned unchanged, so an unexpected shape shows up in the output
/// rather than being silently rewritten into some other path.
fn next_path_from_url(url: &str) -> String {
    let Some((_scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let Some((authority, path)) = rest.split_once('/') else {
        return url.to_owned();
    };
    if !authority.contains("api.") {
        return url.to_owned();
    }
    path.to_owned()
}

/// Split a `gh api --include` response into its header block and its body.
///
/// The first blank line is the boundary. The body is returned with its original
/// bytes intact so a JSON parse failure reports the same offset it would have
/// without the headers attached, which is what keeps the header split from
/// hiding the re-serialisation defect.
fn split_headers(response: &str) -> Option<(&str, &str)> {
    let boundary = response
        .find("\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| response.find("\n\n").map(|index| (index, 2)))?;
    let (headers, body) = response.split_at(boundary.0);
    Some((headers, &body[boundary.1..]))
}

/// Read one page: the body parsed as a JSON array, plus its `rel="next"` link.
fn read_page<T: serde::de::DeserializeOwned>(
    path: &str,
    host: &str,
) -> Result<Page<T>, ReadFailure> {
    let response = read_with_headers(path, host)?;
    let items: Vec<T> =
        serde_json::from_str(&response.body).map_err(|error| ReadFailure::Unparseable {
            detail: format!(
                "{error} (body {} bytes, first bad offset {})",
                response.body.len(),
                error.column()
            ),
        })?;
    Ok(Page {
        items,
        next: next_link(response.link.as_deref()),
    })
}

/// Read every page of a paged listing, following the server's own `next` links.
///
/// # Why this exists
///
/// [`read_json_list`] reads exactly one page. A caller that stops at the first
/// page of a longer listing reports a smaller number than exists and still
/// exits successfully — the silent under-report this tool exists to prevent,
/// arriving through a different door. The only correct end to a traversal is
/// the one the server names, so this follows `rel="next"` until the server
/// stops offering one.
///
/// # Fail-closed behaviour
///
/// A page that cannot be read ends the traversal and yields
/// [`PageOutcome::Failed`] naming the exact page that failed. The pages
/// already read are returned with it, so a caller can report both "here is
/// what I did read" and "here is the page where I stopped" instead of
/// presenting a prefix of the listing as the whole of it.
pub fn read_all_pages<T: serde::de::DeserializeOwned>(path: &str, host: &str) -> PageOutcome<T> {
    let mut items: Vec<T> = Vec::new();
    let mut next_path = Some(path.to_owned());
    let mut pages: u64 = 0;

    while let Some(current) = next_path {
        match read_page::<T>(&current, host) {
            Ok(page) => {
                next_path = page.next.as_deref().map(next_path_from_url);
                items.extend(page.items);
                pages += 1;
            }
            Err(failure) => {
                return PageOutcome::Failed {
                    items,
                    pages,
                    failed_page: current,
                    failure,
                };
            }
        }
    }

    PageOutcome::Complete { items, pages }
}

/// A response body plus the `Link` header that came with it.
struct Response {
    body: String,
    link: Option<String>,
}

/// Read one API path, keeping the `Link` header a listing needs.
///
/// `gh api --include` prefixes the status line and headers to the body. The
/// header block is split off and the remainder is validated exactly as the
/// headerless path validates it: an unparseable body is still
/// [`ReadFailure::Unparseable`], and still not an empty page.
fn read_with_headers(path: &str, host: &str) -> Result<Response, ReadFailure> {
    let mut child = match Command::new("gh")
        .args(["api", "--include", "--hostname", host, path])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Err(ReadFailure::Transport {
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
        return Err(ReadFailure::Transport {
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
            return Err(ReadFailure::Transport {
                detail: format!("could not wait for gh: {error}"),
            });
        }
    };

    if !status.success() {
        return Err(ReadFailure::Transport {
            detail: format!(
                "gh exited {status} for {path}: {}",
                stderr.trim().lines().next_back().unwrap_or("no stderr")
            ),
        });
    }

    let (headers, body) = split_headers(&stdout).ok_or_else(|| ReadFailure::Transport {
        detail: format!("gh exited 0 with no header block for {path}"),
    })?;
    if body.trim().is_empty() {
        return Err(ReadFailure::Transport {
            detail: format!("gh exited 0 with an empty body for {path}"),
        });
    }

    let link = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("link")
            .then(|| value.trim().to_owned())
    });

    Ok(Response {
        body: body.to_owned(),
        link,
    })
}

/// Test-only alias so the paging tests can assert the header/body split
/// without making the split itself part of the crate's public API.
#[doc(hidden)]
pub fn split_headers_for_test(response: &str) -> Option<(&str, &str)> {
    split_headers(response)
}
