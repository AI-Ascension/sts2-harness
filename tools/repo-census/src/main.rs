// SPDX-License-Identifier: MIT

//! Census merged pull requests and their review record, org-wide.
//!
//! The exit status is the contract. A run that could not read everything exits
//! non-zero, so a gate built on this cannot report a clean measurement over a
//! partial read. This is the specific failure `.github#50` describes: an
//! instrument that "fails closed on one object and is retried into silence".

use std::process::ExitCode;

use repo_census::census::{RepoCensus, census_merged_pulls};

const DEFAULT_HOST: &str = "github.com";
const HOST_FLAG: &str = "--host";

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    // The host is an explicit flag rather than a leading positional, so that
    // `repo-census sts2-harness` names one repository instead of silently
    // consuming it as a hostname and censusing nothing.
    let mut host = DEFAULT_HOST.to_owned();
    let mut repositories: Vec<String> = Vec::new();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == HOST_FLAG {
            match raw.get(index + 1) {
                Some(value) => {
                    host = value.clone();
                    index += 2;
                }
                None => {
                    eprintln!("{HOST_FLAG} needs a value");
                    return ExitCode::from(2);
                }
            }
            continue;
        }
        repositories.push(raw[index].clone());
        index += 1;
    }

    if repositories.is_empty() {
        eprintln!("usage: repo-census [--host <host>] <owner/repository>...");
        eprintln!(
            "  reports, per repository, how many merged pull requests carry a review\n\
             \x20 of record pinned to the merged head, and names every object it could\n\
             \x20 not read. Exits non-zero when any object could not be read."
        );
        return ExitCode::from(2);
    }

    let mut total = RepoCensus::default();
    let mut complete = true;

    for repository in &repositories {
        let census = census_merged_pulls(repository, &host);
        println!(
            "{repository}: pages={} merged={} pinned={} unpinned={} absent={} unreadable={}",
            census.pages_read,
            census.classified,
            census.pinned,
            census.unpinned,
            census.absent,
            census.unreadable.len()
        );
        for unreadable in &census.unreadable {
            println!(
                "  UNREADABLE {}: {}",
                unreadable.subject, unreadable.failure
            );
        }
        complete &= census.is_complete();
        total.pages_read += census.pages_read;
        total.classified += census.classified;
        total.pinned += census.pinned;
        total.unpinned += census.unpinned;
        total.absent += census.absent;
        total.unreadable.extend(census.unreadable);
    }

    println!(
        "TOTAL: pages={} merged={} pinned={} unpinned={} absent={} unreadable={}",
        total.pages_read,
        total.classified,
        total.pinned,
        total.unpinned,
        total.absent,
        total.unreadable.len()
    );

    if complete {
        ExitCode::SUCCESS
    } else {
        // Deliberately not a retryable signal: retrying is what made this
        // invisible. The unreadable objects are named above so they can be
        // read another way.
        eprintln!(
            "census incomplete: {} object(s) could not be read; the counts above \
             cover only the {} page(s) that were read",
            total.unreadable.len(),
            total.pages_read
        );
        ExitCode::FAILURE
    }
}
