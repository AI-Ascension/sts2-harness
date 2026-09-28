// SPDX-License-Identifier: MIT

//! An organisation-wide census instrument that cannot silently lose an object.
//!
//! Closes `.github#50` (remedy 3) and supplies the measurement for
//! `.github#49` (remedy: count parse failures and name the objects).

pub mod census;
pub mod paginate;
pub mod transport;
