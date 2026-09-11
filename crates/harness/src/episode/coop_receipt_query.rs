// SPDX-License-Identifier: MIT

#[path = "coop_receipt_query_identity.rs"]
mod identity;
#[path = "coop_receipt_query_result.rs"]
mod result;

pub use identity::{
    ReceiptQueryActionKind, ReceiptQueryCoordinate, ReceiptQueryIdentity,
    ReceiptQueryIdentityError, ReceiptQueryLocation,
};
pub use result::{ReceiptQueryError, ReceiptQueryReceipt, ReceiptQueryResult, ReceiptQueryStatus};
