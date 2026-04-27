//! OL state indexing operation interface.

use strata_db_types::{
    ol_state_index::{
        AccountEpochKey, AccountInboxEntry, AccountUpdateEntry, BlockIndexingWrites,
        EpochIndexingData, EpochIndexingWrites,
    },
    traits::*,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLStateIndexingDatabase> => OLStateIndexingOps, component = components::STORAGE_OL_STATE_INDEXING) {
        apply_epoch_indexing(writes: EpochIndexingWrites) => ();
        apply_block_indexing(writes: BlockIndexingWrites) => ();
        set_epoch_commitment(epoch: Epoch, commitment: EpochCommitment) => ();
        get_epoch_indexing_data(epoch: Epoch) => Option<EpochIndexingData>;
        get_account_update_entry(key: AccountEpochKey) => Option<AccountUpdateEntry>;
        get_account_inbox_entry(key: AccountEpochKey) => Option<AccountInboxEntry>;
        get_account_creation_epoch(account_id: AccountId) => Option<Epoch>;
    }
}
