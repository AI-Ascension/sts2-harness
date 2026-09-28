// SPDX-License-Identifier: MIT

//! The paging surface, exercised from outside the module.
//!
//! These tests use the public API only, so they also pin that the surface a
//! caller needs is genuinely reachable from outside `paginate`.

#[cfg(test)]
mod paging {
    // A test asserts; these lints forbid the assertion forms in the code
    // under test, which is the point of them.
    #![allow(clippy::expect_used, clippy::panic)]

    use repo_census::paginate::{PageOutcome, next_link, split_headers_for_test as split_headers};
    use repo_census::transport::ReadFailure;

    /// Captured verbatim from `gh api -i` against
    /// `repos/AI-Ascension/sts2-harness/pulls?state=closed&per_page=100&page=5`.
    ///
    /// The traversal must follow the server's own idea of the next page, so the
    /// test uses a real header rather than one written to suit the parser.
    const REAL_MIDDLE_PAGE_LINK: &str = "<https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=4>; rel=\"prev\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=6>; rel=\"next\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=6>; rel=\"last\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=1>; rel=\"first\"";

    /// The same endpoint at page 6, the last page.
    const REAL_LAST_PAGE_LINK: &str = "<https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=5>; rel=\"prev\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=1>; rel=\"first\"";

    #[test]
    fn the_next_page_is_read_from_the_servers_own_header() {
        let next = next_link(Some(REAL_MIDDLE_PAGE_LINK)).expect("a next link is present");
        assert!(next.contains("page=6"), "unexpected next page: {next}");
        assert!(next.starts_with("https://api.github.com/"));
    }

    #[test]
    fn the_last_page_offers_no_next_link() {
        assert_eq!(next_link(Some(REAL_LAST_PAGE_LINK)), None);
    }

    #[test]
    fn an_absent_link_header_ends_the_traversal() {
        assert_eq!(next_link(None), None);
        assert_eq!(next_link(Some("")), None);
    }

    #[test]
    fn a_malformed_next_entry_does_not_invent_a_successor() {
        // A `next` entry in some unexpected shape must not be guessed at.
        // Stopping is the safe direction: it loses pages loudly rather than
        // reading a page that was never offered.
        assert_eq!(next_link(Some("garbage")), None);
        assert_eq!(next_link(Some("page=7; rel=\"next\"")), None);
    }

    #[test]
    fn rel_next_is_not_confused_with_a_similarly_named_rel() {
        let header = "<https://api.github.com/x?page=9>; rel=\"next_page\", <https://api.github.com/x?page=1>; rel=\"last\"";
        assert_eq!(next_link(Some(header)), None);
    }

    #[test]
    fn a_comma_inside_a_url_does_not_truncate_the_next_link() {
        // Splitting the header on `,` would cut this URL in half and request a
        // page that does not exist.
        let header = "<https://api.github.com/x?a=1,2&page=3>; rel=\"next\"";
        assert_eq!(
            next_link(Some(header)).as_deref(),
            Some("https://api.github.com/x?a=1,2&page=3")
        );
    }

    #[test]
    fn a_next_link_is_found_when_earlier_entries_carry_a_semicolon() {
        // `rel="next"` on the first page of the real listing sits behind a
        // `prev` entry whose URL and parameters must not be absorbed into it.
        // The captured page-1 header is the regression case: a parser that
        // folds the following URL into the current entry's parameters loses the
        // `rel="next"` marker and silently stops after one page.
        let header = "<https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=1>; rel=\"first\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=2>; rel=\"next\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=6>; rel=\"last\"";
        let next = next_link(Some(header)).expect("next is present");
        assert!(next.ends_with("page=2"), "unexpected next page: {next}");
    }

    #[test]
    fn a_first_page_next_link_is_read_from_the_real_header_shape() {
        // Verbatim page-1 Link header, where `next` is the *first* entry and no
        // `prev` precedes it.
        let header = "<https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=2>; rel=\"next\", <https://api.github.com/repositories/1354378100/pulls?state=closed&per_page=100&page=6>; rel=\"last\"";
        let next = next_link(Some(header)).expect("next is present");
        assert!(next.ends_with("page=2"), "unexpected next page: {next}");
    }

    #[test]
    fn the_header_block_is_split_off_without_touching_the_body() {
        let response = "HTTP/2.0 200 OK\r\nLink: <https://api.github.com/x?page=2>; rel=\"next\"\r\n\r\n[{\"a\":1}]";
        let (headers, body) = split_headers(response).expect("a header block");
        assert!(headers.starts_with("HTTP/2.0 200 OK"));
        assert!(headers.contains("rel=\"next\""));
        assert_eq!(body, "[{\"a\":1}]");
    }

    #[test]
    fn an_unterminated_header_block_is_not_treated_as_a_body() {
        assert_eq!(split_headers("HTTP/2.0 200 OK\r\nLink: x"), None);
    }

    #[test]
    fn an_incomplete_traversal_exposes_both_the_prefix_and_the_failed_page() {
        // The shape the census must not be able to pass off as a measurement:
        // some rows were read, and the page after them could not be.
        let outcome: PageOutcome<u8> = PageOutcome::Failed {
            items: vec![1, 2],
            pages: 1,
            failed_page: "repos/x/pulls?page=2".to_owned(),
            failure: ReadFailure::Unparseable {
                detail: "boom".to_owned(),
            },
        };
        assert_eq!(outcome.items(), &[1, 2]);
        assert!(!outcome.is_complete());
        match outcome {
            PageOutcome::Failed { failed_page, .. } => {
                assert_eq!(failed_page, "repos/x/pulls?page=2");
            }
            PageOutcome::Complete { .. } => panic!("expected a failed traversal"),
        }
    }

    #[test]
    fn a_complete_traversal_reports_completeness() {
        let outcome: PageOutcome<u8> = PageOutcome::Complete {
            items: vec![1, 2, 3],
            pages: 3,
        };
        assert!(outcome.is_complete());
        assert_eq!(outcome.items(), &[1, 2, 3]);
        assert_eq!(outcome.pages(), 3);
    }
}
