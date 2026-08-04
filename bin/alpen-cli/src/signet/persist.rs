use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::OnceLock};

use bdk_wallet::{
    rusqlite::{self, Connection, OptionalExtension},
    ChangeSet, WalletPersister,
};

use crate::signet::SignetWallet;

/// Wrapper around the built-in rusqlite db.
///
/// It allows [`PersistedWallet`](crate::signet::PersistedWallet) to be shared across multiple
/// threads by lazily initializing per core connections to the sqlite db and keeping them in
/// local thread storage instead of sharing the connection across cores.
///
/// WARNING: [`set_data_dir`] **MUST** be called and set before using [`Persister`].
#[derive(Debug)]
pub struct Persister;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Sets the data directory static for the thread local DB.
///
/// Must be called before accessing [`Persister`].
///
/// Can only be set once - will return whether value was set.
pub fn set_data_dir(data_dir: PathBuf) -> bool {
    DATA_DIR.set(data_dir).is_ok()
}

thread_local! {
    static DB: Rc<RefCell<Connection>> = RefCell::new(Connection::open(SignetWallet::db_path("default", DATA_DIR.get().expect("data dir to be set"))).unwrap()).into();
}

impl Persister {
    fn db() -> Rc<RefCell<Connection>> {
        DB.with(|db| db.clone())
    }
}

impl WalletPersister for Persister {
    type Error = rusqlite::Error;

    fn initialize(_persister: &mut Self) -> Result<bdk_wallet::ChangeSet, Self::Error> {
        let db = Self::db();
        let mut db_ref = db.borrow_mut();
        let db_tx = db_ref.transaction()?;
        ensure_full_scan_state_table(&db_tx)?;
        ChangeSet::init_sqlite_tables(&db_tx)?;
        let changeset = ChangeSet::from_sqlite(&db_tx)?;
        db_tx.commit()?;
        Ok(changeset)
    }

    fn persist(
        _persister: &mut Self,
        changeset: &bdk_wallet::ChangeSet,
    ) -> Result<(), Self::Error> {
        let db = Self::db();
        let mut db_ref = db.borrow_mut();
        let db_tx = db_ref.transaction()?;
        changeset.persist_to_sqlite(&db_tx)?;
        db_tx.commit()
    }
}

impl Persister {
    /// True once a full scan has completed at least once for this wallet.
    ///
    /// This is tracked independently of revealed addresses: some code
    /// paths (e.g. `faucet` with no address argument) reveal an address
    /// without ever syncing, which would otherwise look identical to a
    /// wallet that has actually been scanned.
    ///
    /// This creates its own table if needed, so callers may use it before
    /// the wallet itself is loaded.
    pub fn full_scan_completed() -> Result<bool, rusqlite::Error> {
        let db = Self::db();
        let db_ref = db.borrow();
        ensure_full_scan_state_table(&db_ref)?;
        full_scan_completed(&db_ref)
    }

    /// Records that a full scan has completed for this wallet.
    pub fn mark_full_scan_completed() -> Result<(), rusqlite::Error> {
        let db = Self::db();
        let db_ref = db.borrow();
        ensure_full_scan_state_table(&db_ref)?;
        mark_full_scan_completed(&db_ref)
    }
}

const CREATE_FULL_SCAN_STATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS full_scan_state (
    id INTEGER PRIMARY KEY CHECK (id = 0),
    completed INTEGER NOT NULL
)";

fn ensure_full_scan_state_table(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute(CREATE_FULL_SCAN_STATE_TABLE, []).map(|_| ())
}

fn full_scan_completed(conn: &Connection) -> Result<bool, rusqlite::Error> {
    let completed: Option<bool> = conn
        .query_row(
            "SELECT completed FROM full_scan_state WHERE id = 0",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(completed.unwrap_or(false))
}

fn mark_full_scan_completed(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO full_scan_state (id, completed) VALUES (0, TRUE)
         ON CONFLICT(id) DO UPDATE SET completed = TRUE",
        [],
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use bdk_wallet::rusqlite::Connection;

    use super::{ensure_full_scan_state_table, full_scan_completed, mark_full_scan_completed};

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        ensure_full_scan_state_table(&conn).expect("table should be created");
        conn
    }

    #[test]
    fn test_full_scan_completed_is_false_before_any_scan() {
        let conn = test_conn();

        assert!(!full_scan_completed(&conn).unwrap());
    }

    #[test]
    fn test_full_scan_completed_is_true_after_marking() {
        let conn = test_conn();

        mark_full_scan_completed(&conn).unwrap();

        assert!(full_scan_completed(&conn).unwrap());
    }

    #[test]
    fn test_mark_full_scan_completed_is_idempotent() {
        let conn = test_conn();

        mark_full_scan_completed(&conn).unwrap();
        mark_full_scan_completed(&conn).unwrap();

        assert!(full_scan_completed(&conn).unwrap());
    }

    #[test]
    fn test_ensure_table_preserves_an_existing_flag() {
        let conn = test_conn();
        mark_full_scan_completed(&conn).unwrap();

        // Runs on every open, including for wallets created before this
        // table existed, so it must never clear what is already recorded.
        ensure_full_scan_state_table(&conn).unwrap();

        assert!(full_scan_completed(&conn).unwrap());
    }
}
