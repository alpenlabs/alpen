//! OL state indexing operation interface.

use strata_db_types::{
    ol_state_index::{AccountUpdateRecord, EpochIndexingData, InboxMessageRecord, IndexingWrites},
    traits::*,
};
use strata_identifiers::{AccountId, Epoch, EpochCommitment, OLBlockCommitment};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: OLStateIndexingDatabase> => OLStateIndexingOps, component = components::STORAGE_OL_STATE_INDEXING) {
        apply_epoch_indexing(commitment: EpochCommitment, writes: IndexingWrites) => ();
        apply_block_indexing(epoch: Epoch, block: OLBlockCommitment, writes: IndexingWrites) => ();
        set_epoch_commitment(epoch: Epoch, commitment: EpochCommitment) => ();
        get_epoch_indexing_data(epoch: Epoch) => Option<EpochIndexingData>;
        get_account_update_records(epoch: Epoch, account: AccountId) => Option<Vec<AccountUpdateRecord>>;
        get_account_inbox_records(epoch: Epoch, account: AccountId) => Option<Vec<InboxMessageRecord>>;
        get_account_creation_epoch(account_id: AccountId) => Option<Epoch>;
    }
}
