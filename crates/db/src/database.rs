use std::sync::Arc;

use super::traits::*;

/// Shim database type to wrap various database impls.
///
/// It assumes that all the database impls are wrapped in `Arc`s and that
/// the provider and stores are actually the same types. We might actually
/// use this in practice, it's just for testing.
#[derive(Debug)]
pub struct CommonDatabase<L1DB, L2DB, SyncEventDB, ClientStateDB, ChainstateDB, CheckpointDB>
where
    L1DB: L1Database + Sync + Send + 'static,
    L2DB: L2BlockDatabase + Sync + Send + 'static,
    SyncEventDB: SyncEventDatabase + Sync + Send + 'static,
    ClientStateDB: ClientStateDatabase + Sync + Send + 'static,
    ChainstateDB: ChainstateDatabase + Sync + Send + 'static,
    CheckpointDB: CheckpointDatabase + Sync + Send + 'static,
{
    l1_db: Arc<L1DB>,
    l2_db: Arc<L2DB>,
    sync_event_db: Arc<SyncEventDB>,
    client_state_db: Arc<ClientStateDB>,
    chain_state_db: Arc<ChainstateDB>,
    checkpoint_db: Arc<CheckpointDB>,
}

impl<L1DB, L2DB, SyncEventDB, ClientStateDB, ChainstateDB, CheckpointDB>
    CommonDatabase<L1DB, L2DB, SyncEventDB, ClientStateDB, ChainstateDB, CheckpointDB>
where
    L1DB: L1Database + Sync + Send + 'static,
    L2DB: L2BlockDatabase + Sync + Send + 'static,
    SyncEventDB: SyncEventDatabase + Sync + Send + 'static,
    ClientStateDB: ClientStateDatabase + Sync + Send + 'static,
    ChainstateDB: ChainstateDatabase + Sync + Send + 'static,
    CheckpointDB: CheckpointDatabase + Sync + Send + 'static,
{
    pub fn new(
        l1_db: Arc<L1DB>,
        l2_db: Arc<L2DB>,
        sync_event_db: Arc<SyncEventDB>,
        client_state_db: Arc<ClientStateDB>,
        chain_state_db: Arc<ChainstateDB>,
        checkpoint_db: Arc<CheckpointDB>,
    ) -> Self {
        Self {
            l1_db,
            l2_db,
            sync_event_db,
            client_state_db,
            chain_state_db,
            checkpoint_db,
        }
    }
}

impl<L1DB, L2DB, SyncEventDB, ClientStateDB, ChainstateDB, CheckpointDB> Database
    for CommonDatabase<L1DB, L2DB, SyncEventDB, ClientStateDB, ChainstateDB, CheckpointDB>
where
    L1DB: L1Database + Sync + Send + 'static,
    L2DB: L2BlockDatabase + Sync + Send + 'static,
    SyncEventDB: SyncEventDatabase + Sync + Send + 'static,
    ClientStateDB: ClientStateDatabase + Sync + Send + 'static,
    ChainstateDB: ChainstateDatabase + Sync + Send + 'static,
    CheckpointDB: CheckpointDatabase + Sync + Send + 'static,
{
    type L1DB = L1DB;
    type L2DB = L2DB;
    type SyncEventDB = SyncEventDB;
    type ClientStateDB = ClientStateDB;
    type ChainstateDB = ChainstateDB;
    type CheckpointDB = CheckpointDB;

    fn l1_db(&self) -> &Arc<Self::L1DB> {
        &self.l1_db
    }

    fn l2_db(&self) -> &Arc<Self::L2DB> {
        &self.l2_db
    }

    fn sync_event_db(&self) -> &Arc<Self::SyncEventDB> {
        &self.sync_event_db
    }

    fn client_state_db(&self) -> &Arc<Self::ClientStateDB> {
        &self.client_state_db
    }

    fn chain_state_db(&self) -> &Arc<Self::ChainstateDB> {
        &self.chain_state_db
    }

    fn checkpoint_db(&self) -> &Arc<Self::CheckpointDB> {
        &self.checkpoint_db
    }
}
