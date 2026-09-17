// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::path::Path;
use sts2_harness::context_memory::{DurableMemoryStore, MemoryScope};

pub(super) fn assert_archive_transcript(
    replay: bool,
    scripted: bool,
    mcp_events: &[Value],
    archive_path: &Path,
    scope: &MemoryScope,
) {
    if replay {
        assert!(
            !mcp_events.iter().any(|event| {
                matches!(
                    event["tool"].as_str(),
                    Some("sts2.game_information_capabilities" | "sts2.game_information_list")
                )
            }),
            "replay must deliver the archived transcript without another game-information query"
        );
    } else {
        if scripted {
            assert!(
                mcp_events
                    .iter()
                    .any(|event| event["tool"] == "sts2.game_information_list"),
                "the live entry must send the admitted query through the actual MCP process"
            );
        }
        let archived = DurableMemoryStore::open_private(
            archive_path.to_str().expect("UTF-8 archive path"),
            scope.clone(),
            [11; 32],
        )
        .expect("reopen encrypted lookup archive");
        let archived_corpus = archived.load_corpus().expect("read archived transcript");
        assert!(
            archived_corpus
                .entries()
                .any(|entry| entry.entry_id.starts_with("lookup-archive:")),
            "normal runtime shutdown must persist the game-information archive"
        );
    }
}
