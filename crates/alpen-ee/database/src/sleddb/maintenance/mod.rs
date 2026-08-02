//! Startup maintenance required before EE node services use the database.

mod jobs;

use super::EeNodeDBSled;
use crate::DbResult;

/// Runs the explicit set of maintenance jobs required by this binary.
pub(super) fn run_startup_jobs(db: &EeNodeDBSled) -> DbResult<()> {
    jobs::chunk_work_indexes::run_if_needed(db)?;
    Ok(())
}
