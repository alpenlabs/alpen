//! Account genesis data operation interface.

use strata_db_types::{traits::*, types::AccountExtraData};
use strata_identifiers::{AccountId, Epoch};
use strata_primitives::nonempty_vec::NonEmptyVec;

use crate::{exec::*, instrumentation::components};

inst_ops_simple! {
    (<D: AccountDatabase> => AccountOps, component = components::STORAGE_ACCOUNT_GENESIS) {
        insert_account_creation_epoch(account_id: AccountId, epoch: Epoch) => ();
        get_account_creation_epoch(account_id: AccountId) => Option<Epoch>;
        append_account_extra_data(key: (AccountId, Epoch), extra_data: AccountExtraData) => ();
        put_account_extra_data(key: (AccountId, Epoch), extra_data: NonEmptyVec<AccountExtraData>) => ();
        get_account_extra_data(key: (AccountId, Epoch)) => Option<NonEmptyVec<AccountExtraData>>;
    }
}
