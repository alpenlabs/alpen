mod task;

use std::num::NonZeroU8;

use strata_acct_types::AccountId;
pub use task::block_builder_task;

#[derive(Debug)]
pub struct BlockBuilderConfig {
    pub blocktime_ms: u64,
    pub max_deposits_per_block: NonZeroU8,
    pub bridge_gateway_account_id: AccountId,
}
