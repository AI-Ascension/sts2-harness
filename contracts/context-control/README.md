# Context-control contract pin

The companion harness consumes the same immutable Phase 2 schema artifacts as the target
console. The executable Rust boundary is in crates/harness/src/context_control; the schemas here
are the reviewable wire pin.

Target main pin: 2e1bfe0d4e62ac7f1efcefe4b1cc940182146880

Harness main pin: 780f2d521508a2aadc76c4d779544d967955f102

Run sha256sum against the corresponding target contract directory when reviewing a cross-repo
change. The two directories must remain byte-identical.
