use std::path::{Path, PathBuf};

use eyre::Context;
pub use sleddb::EeDatabases;
use tokio::task;

use crate::sleddb;

/// Opens the EE databases and completes required startup maintenance before returning.
///
/// Sled recovery and startup jobs may block, so the complete open runs on Tokio's blocking pool.
/// A successfully returned database is ready for node services to use.
pub async fn open_for_node(datadir: PathBuf, db_retry_count: u16) -> eyre::Result<EeDatabases> {
    run_blocking(move || sleddb::open_database_for_node(&datadir, db_retry_count)).await
}

/// Opens the EE databases without running startup maintenance.
///
/// Intended for offline inspection and dry-run tooling. Opening Sled may still perform its own
/// housekeeping, but this path does not deliberately rebuild derived state or claim maintenance
/// completion.
pub fn open_for_offline_tooling(datadir: &Path, db_retry_count: u16) -> eyre::Result<EeDatabases> {
    sleddb::open_database(datadir, db_retry_count)
}

async fn run_blocking<T>(
    operation: impl FnOnce() -> eyre::Result<T> + Send + 'static,
) -> eyre::Result<T>
where
    T: Send + 'static,
{
    task::spawn_blocking(operation)
        .await
        .wrap_err("EE database open task panicked")?
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread,
        time::Duration,
    };

    use tokio::time;

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_adapter_keeps_the_runtime_thread_responsive() {
        let heartbeat_observed = Arc::new(AtomicBool::new(false));
        let observed_from_worker = heartbeat_observed.clone();

        let heartbeat = async {
            time::sleep(Duration::from_millis(10)).await;
            heartbeat_observed.store(true, Ordering::SeqCst);
        };
        let blocking = run_blocking(move || {
            thread::sleep(Duration::from_millis(50));
            assert!(observed_from_worker.load(Ordering::SeqCst));
            Ok(())
        });

        let ((), result) = tokio::join!(heartbeat, blocking);
        result.unwrap();
    }
}
