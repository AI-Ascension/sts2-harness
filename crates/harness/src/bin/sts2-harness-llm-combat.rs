// SPDX-License-Identifier: MIT

//! Entry-point marker for the LLM combat coordinator.
//!
//! The real transport and licensed game launch are injected by a later runtime integration. This
//! binary intentionally does not invent a gameplay action when those dependencies are absent.

fn main() {
    println!(
        "LLM combat coordinator requires a configured Exo transport and gateway episode; status: unverified"
    );
}
