//! Reusable utilities.

use std::time;

/// Returns the current time in milliseconds since UNIX_EPOCH.
pub fn now_millis() -> u64 {
    time::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64
}
