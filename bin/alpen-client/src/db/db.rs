use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{OLBlockCommitment, OLBlockId};
use strata_storage_common::{inst_ops_ctx_shim_generic, inst_ops_generic};

use crate::{
    db::{error::DbError, DbResult},
    traits::storage::EeAccountStateAtBlock,
};

pub(crate) trait EeNodeDb: Send + Sync + 'static {
    fn store_ee_account_state(
        &self,
        ol_block: OLBlockCommitment,
        ee_account_state: EeAccountState,
    ) -> DbResult<()>;

    fn rollback_ee_account_state(&self, to_slot: u64) -> DbResult<()>;

    fn get_ol_blockid(&self, slot: u64) -> DbResult<Option<OLBlockId>>;

    fn ee_account_state(&self, block_id: OLBlockId) -> DbResult<Option<EeAccountStateAtBlock>>;

    fn best_ee_account_state(&self) -> DbResult<Option<EeAccountStateAtBlock>>;
}

pub(crate) mod ops {
    use super::*;

    inst_ops_generic! {
        (<D: EeNodeDb> => EeNodeOps, DbError) {
            store_ee_account_state(ol_block: OLBlockCommitment, ee_account_state: EeAccountState) =>();
            rollback_ee_account_state(to_slot: u64) => ();
            get_ol_blockid(slot: u64) => Option<OLBlockId>;
            ee_account_state(block_id: OLBlockId) => Option<EeAccountStateAtBlock>;
            best_ee_account_state() => Option<EeAccountStateAtBlock>;
        }
    }
}
