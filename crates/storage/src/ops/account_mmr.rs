//! Account-scoped MMR database operation interface.

use strata_db_types::traits::AccountMmrDatabase;
use strata_identifiers::AccountId;

use crate::exec::*;

inst_ops_simple! {
    (<D: AccountMmrDatabase> => AccountMmrDataOps) {
        append_leaf(account: AccountId, hash: [u8; 32]) => u64;
        pop_leaf(account: AccountId) => Option<[u8; 32]>;
        get_node(account: AccountId, pos: u64) => [u8; 32];
        mmr_size(account: AccountId) => u64;
        num_leaves(account: AccountId) => u64;
    }
}
