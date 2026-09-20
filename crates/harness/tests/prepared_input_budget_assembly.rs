// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;

#[test]
fn assembled_input_admission_reuses_the_request_bound_and_reserve_arithmetic() {
    // The served managed path admits bytes that already exist, so it shares this arithmetic with
    // the request path instead of re-deriving the subtraction beside it. A drift between the two
    // shapes would show up here as a different boundary or a different refusal.
    let reserve = 100usize;
    let input_bytes = 512usize;
    let bound = AssembledInputBound::new(input_bytes + reserve, reserve).expect("assembled bound");
    assert_eq!(bound.input_headroom().expect("headroom"), input_bytes);
    assert_eq!(
        bound.admit(input_bytes).expect("exact boundary"),
        input_bytes + reserve,
        "the admitted whole bytes include the reserve"
    );

    let one_over =
        AssembledInputBound::new(input_bytes + reserve - 1, reserve).expect("assembled bound");
    assert_eq!(
        one_over.admit(input_bytes).expect_err("one over refuses"),
        PreparedBudgetError::CombinedWindowOverflow {
            input_bytes,
            output_reserve_bytes: reserve,
            effective: input_bytes + reserve - 1,
        }
    );

    // A reserve that cannot fit inside the bound refuses on the same axis `prepare` uses, and
    // neither a zero bound nor a zero reserve is treated as unlimited.
    let reserve_over_bound = AssembledInputBound::new(64, 65).expect("assembled bound");
    assert_eq!(
        reserve_over_bound
            .admit(0)
            .expect_err("reserve exceeds the bound"),
        PreparedBudgetError::OutputReserveOverflow {
            requested: 65,
            effective: 64,
        }
    );
    assert_eq!(
        AssembledInputBound::new(0, reserve).expect_err("zero bound"),
        PreparedBudgetError::InvalidRequest("whole_input_byte_bound")
    );
    assert_eq!(
        AssembledInputBound::new(1024, 0).expect_err("zero reserve"),
        PreparedBudgetError::InvalidRequest("output_reserve_bytes")
    );
    assert_eq!(
        AssembledInputBound::new(1024, MAX_PREPARED_OUTPUT_RESERVE_BYTES + 1)
            .expect_err("reserve above the harness guard"),
        PreparedBudgetError::InvalidRequest("output_reserve_bytes")
    );
}
