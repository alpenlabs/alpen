//! Account genesis data operation interface.

use strata_db_types::traits::*;
use strata_identifiers::{AccountId, Epoch, OLBlockId};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: AccountDatabase> => AccountOps, component = components::STORAGE_ACCOUNT_GENESIS) {
        insert_account_creation_epoch(account_id: AccountId, epoch: Epoch) => ();
        get_account_creation_epoch(account_id: AccountId) => Option<Epoch>;
        insert_account_extra_data(key: (AccountId, OLBlockId), extra_data: Vec<u8>) => ();
        get_account_extra_data(key: (AccountId, OLBlockId)) => Option<Vec<u8>>;
    }
}
