//! driftkit: finds where a project's declarations and its behaviour disagree.
//!
//! The library half. Each check lives in its own module and obeys the
//! contract in `core`: a boolean `hard` on every finding, a `=== Coverage ===`
//! block that says what was compared, and a binding counter, because a scan
//! that matched 3 of 40 must not look healthy.

pub mod core;
pub mod env;
pub mod mcp;
