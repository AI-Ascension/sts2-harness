// SPDX-License-Identifier: MIT

use std::io::{self, BufRead, Write};
use sts2_harness::context_memory::{FakeSummaryPeer, run_fake_peer_line};

fn main() {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let stdin = io::stdin();
    let mut peer = match FakeSummaryPeer::new(b"fake-summary-requires-review".to_vec()) {
        Ok(peer) => peer,
        Err(error) => {
            eprintln!("fake peer setup failed: {error}");
            std::process::exit(2);
        }
    };
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            std::process::exit(2);
        };
        let result = run_fake_peer_line(line.as_bytes(), &mut peer).and_then(|response| {
            serde_json::to_vec(&response)
                .map_err(|_| sts2_harness::context_memory::MemoryError::Unsupported)
        });
        match result {
            Ok(bytes) => {
                if output.write_all(&bytes).is_err() || output.write_all(b"\n").is_err() {
                    std::process::exit(2);
                }
            }
            Err(error) => {
                let body = serde_json::json!({
                    "schema": "ascension.context-memory.fake-peer-error.v1",
                    "error": error.to_string(),
                });
                if writeln!(output, "{body}").is_err() {
                    std::process::exit(2);
                }
            }
        }
    }
}
