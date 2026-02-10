//! Account genesis data operation interface.

use strata_db_types::traits::*;
use strata_identifiers::{AccountId, Epoch};

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: AccountGenesisDatabase> => AccountGenesisOps, component = components::STORAGE_ACCOUNT_GENESIS) {
        insert_account_creation_epoch(account_id: AccountId, epoch: Epoch) => ();
        get_account_creation_epoch(account_id: AccountId) => Option<Epoch>;
    }
}
