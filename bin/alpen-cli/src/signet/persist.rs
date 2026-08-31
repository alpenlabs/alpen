use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::OnceLock};

use bdk_wallet::{
    bitcoin::Network,
    rusqlite::{self, Connection},
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

static PERSISTENCE: OnceLock<(PathBuf, Network)> = OnceLock::new();

/// Sets the data directory static for the thread local DB.
///
/// Must be called before accessing [`Persister`].
///
/// Can only be set once - will return whether value was set.
pub fn set_data_dir(data_dir: PathBuf, network: Network) -> bool {
    PERSISTENCE.set((data_dir, network)).is_ok()
}

thread_local! {
    static DB: Rc<RefCell<Connection>> = {
        let (data_dir, network) = PERSISTENCE.get().expect("persistence to be configured");
        RefCell::new(Connection::open(SignetWallet::db_path("default", data_dir, *network)).unwrap()).into()
    };
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
