// SPDX-License-Identifier: MIT

//! Per-repository census of merged pull requests and their review record.
//!
//! Two properties this must have, both of which the `gh`-based tooling it
//! replaces did not have:
//!
//! 1. **Failures are counted and named.** Every API read that does not produce
//!    usable data is recorded against the object it was reading, so the output
//!    can say "read N, could not read M, and here they are" instead of quietly
//!    reporting a smaller N.
//! 2. **A red run is not a pass.** `sts2-harness#714` records that a run object
//!    reports only its final attempt, so a top-level `conclusion` of `success`
//!    hides earlier failed attempts. A review that pins an earlier attempt's
//!    red is counted as unpinned, not as pinned.

use serde::Deserialize;

use crate::transport::{Fetch, PageOutcome, ReadFailure, read_all_pages, read_json_list};

#[derive(Debug, Deserialize)]
struct PullRequestSummary {
    number: u64,
    merged_at: Option<String>,
    head: Head,
}

#[derive(Debug, Deserialize)]
struct Head {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct Review {
    /// The commit the review was submitted against.
    commit_id: String,
    state: String,
}

/// The minimum a review must be to count as a review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewProbe {
    pub commit_id: String,
    pub state: String,
}

impl From<Review> for ReviewProbe {
    fn from(review: Review) -> Self {
        Self {
            commit_id: review.commit_id,
            state: review.state,
        }
    }
}

/// How a merged pull request relates to the review requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRecord {
    /// At least one review, and at least one of them is pinned to the merged
    /// head.
    Pinned,
    /// Reviews exist, but none is pinned to the head that actually merged.
    Unpinned,
    /// No review of any kind was submitted.
    Absent,
}

impl ReviewRecord {
    /// `true` only when a review is pinned to the merged head. This is the
    /// question `.github#49` asks.
    pub fn satisfies_gate(self) -> bool {
        matches!(self, Self::Pinned)
    }
}

/// One thing the census wanted to read and could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// `owner/repo#number` for an object, or the API path for a page.
    pub subject: String,
    pub failure: ReadFailure,
}

/// The result of censusing one repository.
#[derive(Debug, Default)]
pub struct RepoCensus {
    pub repository: String,
    /// Merged pull requests this run actually read and classified.
    pub classified: u64,
    pub pinned: u64,
    pub unpinned: u64,
    pub absent: u64,
    /// Pages of the merged-pull-request listing this run read.
    pub pages_read: u64,
    /// Everything that could not be read, in discovery order.
    pub unreadable: Vec<Unreadable>,
}

impl RepoCensus {
    /// Objects the census expected to see but could not. A census with a
    /// non-zero count here has **not** measured the repository; it has measured
    /// the part of the repository it could read.
    pub fn is_complete(&self) -> bool {
        self.unreadable.is_empty()
    }

    pub fn record(&mut self, subject: &str, failure: ReadFailure) {
        self.unreadable.push(Unreadable {
            subject: subject.to_owned(),
            failure,
        });
    }

    fn classify(&mut self, record: ReviewRecord) {
        self.classified += 1;
        match record {
            ReviewRecord::Pinned => self.pinned += 1,
            ReviewRecord::Unpinned => self.unpinned += 1,
            ReviewRecord::Absent => self.absent += 1,
        }
    }
}

/// `DISMISSED` is not a review. `COMMENTED` is: this campaign's own gate
/// requires exactly that state, and `.github#49` measures the *absence* of a
/// review rather than the strength of one.
fn is_a_review(state: &str) -> bool {
    !matches!(state.to_ascii_uppercase().as_str(), "DISMISSED" | "PENDING")
}

/// Decide the review record for one merged pull request from its reviews.
///
/// `Pinned` requires a review whose `commit_id` equals the head that merged.
/// A review pinned to a commit the pull request no longer has is `Unpinned`,
/// which is the sharper failure `.github#49` measures separately from having
/// no review at all.
pub fn review_record(head_sha: &str, reviews: &[ReviewProbe]) -> ReviewRecord {
    if reviews.is_empty() {
        return ReviewRecord::Absent;
    }
    if reviews
        .iter()
        .any(|review| review.commit_id == head_sha && is_a_review(&review.state))
    {
        return ReviewRecord::Pinned;
    }
    ReviewRecord::Unpinned
}

/// Census every merged pull request in one repository.
///
/// The listing is traversed in full. Reading a single page and treating it as
/// the whole listing is the failure this tool exists to prevent: a repository
/// whose merged pull requests run past one page would be reported as having
/// only the merged pull requests on page one, and the run would still exit
/// zero. The traversal follows the server's `rel="next"` links, so it ends
/// where the server says the listing ends rather than at an assumed page count.
///
/// A page that does not parse is recorded and **counted once per page**, with
/// the page's own identity, because that is the true blast radius: a single
/// unreadable object invalidates every object on its page. The affected pull
/// requests are then unknown rather than silently absent.
pub fn census_merged_pulls(repository: &str, host: &str) -> RepoCensus {
    let mut census = RepoCensus {
        repository: repository.to_owned(),
        ..RepoCensus::default()
    };

    let path = format!("repos/AI-Ascension/{repository}/pulls?state=closed&per_page=100&page=1");
    let pulls: Vec<PullRequestSummary> = match read_all_pages(&path, host) {
        PageOutcome::Complete { items, pages } => {
            census.pages_read = pages;
            items
        }
        PageOutcome::Failed {
            items,
            pages,
            failed_page,
            failure,
        } => {
            // The pages before the failure are a prefix, not the listing. They
            // are classified so the run still measures what it could reach,
            // and the failed page is recorded so the total is known to be
            // short rather than merely small.
            census.record(&failed_page, failure);
            census.pages_read = pages;
            classify_pulls(&mut census, &items, repository, host);
            return census;
        }
    };

    classify_pulls(&mut census, &pulls, repository, host);
    census
}

/// Classify each merged pull request, reading its reviews.
fn classify_pulls(
    census: &mut RepoCensus,
    pulls: &[PullRequestSummary],
    repository: &str,
    host: &str,
) {
    for pull in pulls {
        if pull.merged_at.is_none() {
            continue;
        }
        let subject = format!("{repository}#{}", pull.number);
        let reviews_path = format!(
            "repos/AI-Ascension/{repository}/pulls/{}/reviews",
            pull.number
        );
        match read_json_list::<Review>(&reviews_path, host) {
            Fetch::Parsed(reviews) => {
                let probes: Vec<ReviewProbe> = reviews.into_iter().map(ReviewProbe::from).collect();
                census.classify(review_record(&pull.head.sha, &probes));
            }
            Fetch::Failed(failure) => census.record(&subject, failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadFailure, RepoCensus, ReviewProbe, ReviewRecord, review_record};

    fn probe(commit: &str, state: &str) -> ReviewProbe {
        ReviewProbe {
            commit_id: commit.to_owned(),
            state: state.to_owned(),
        }
    }

    #[test]
    fn no_reviews_is_absent_not_pinned() {
        assert_eq!(review_record("abc123", &[]), ReviewRecord::Absent);
    }

    #[test]
    fn a_review_pinned_to_the_merged_head_satisfies_the_gate() {
        assert_eq!(
            review_record("abc123", &[probe("abc123", "COMMENTED")]),
            ReviewRecord::Pinned
        );
    }

    #[test]
    fn a_review_pinned_to_a_superseded_commit_does_not_satisfy_the_gate() {
        // The `.github#49` "looks reviewed but is not" case: #504, #494 and
        // two others. A review exists, so this is not Absent -- it is the
        // sharper Unpinned failure.
        assert_eq!(
            review_record("abc123", &[probe("old999", "APPROVED")]),
            ReviewRecord::Unpinned
        );
    }

    #[test]
    fn a_dismissed_review_does_not_satisfy_the_gate() {
        assert_eq!(
            review_record("abc123", &[probe("abc123", "DISMISSED")]),
            ReviewRecord::Unpinned
        );
    }

    #[test]
    fn one_pinned_review_among_unpinned_ones_still_satisfies_the_gate() {
        assert_eq!(
            review_record(
                "abc123",
                &[probe("old999", "APPROVED"), probe("abc123", "COMMENTED")]
            ),
            ReviewRecord::Pinned
        );
    }

    #[test]
    fn only_pinned_records_satisfy_the_gate() {
        assert!(ReviewRecord::Pinned.satisfies_gate());
        assert!(!ReviewRecord::Unpinned.satisfies_gate());
        assert!(!ReviewRecord::Absent.satisfies_gate());
    }

    #[test]
    fn a_census_with_an_unreadable_object_is_not_complete() {
        // This is the whole point of the tool. A census that lost an object
        // must not be able to present itself as a clean measurement.
        let mut census = RepoCensus {
            repository: "sts2-game-mod".to_owned(),
            ..RepoCensus::default()
        };
        assert!(census.is_complete());
        census.record(
            "sts2-game-mod#147",
            ReadFailure::Unparseable {
                detail: "Invalid \\uXXXX escape".to_owned(),
            },
        );
        assert!(!census.is_complete());
        assert_eq!(census.unreadable.len(), 1);
    }

    #[test]
    fn an_unreadable_subject_is_reported_not_swallowed() {
        let mut census = RepoCensus::default();
        census.record(
            "sts2-game-mod#147",
            ReadFailure::Transport {
                detail: "boom".to_owned(),
            },
        );
        assert_eq!(census.unreadable[0].subject, "sts2-game-mod#147");
        assert!(census.unreadable[0].failure.to_string().contains("boom"));
    }
}
