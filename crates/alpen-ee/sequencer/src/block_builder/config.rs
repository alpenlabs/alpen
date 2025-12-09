use std::num::NonZeroU8;

use strata_acct_types::AccountId;

#[derive(Debug)]
pub struct BlockBuilderConfig {
    /// Target blocktime in ms
    blocktime_ms: u64,
    /// Max number of deposits that can be processed in a single EE block.
    max_deposits_per_block: NonZeroU8,
    /// [`AccountId`] of bridge gateway on OL.
    bridge_gateway_account_id: AccountId,
}

impl BlockBuilderConfig {
    pub fn new(
        blocktime_ms: u64,
        max_deposits_per_block: NonZeroU8,
        bridge_gateway_account_id: AccountId,
    ) -> Self {
        Self {
            blocktime_ms,
            max_deposits_per_block,
            bridge_gateway_account_id,
        }
    }

    pub fn blocktime_ms(&self) -> u64 {
        self.blocktime_ms
    }

    pub fn max_deposits_per_block(&self) -> NonZeroU8 {
        self.max_deposits_per_block
    }

    pub fn bridge_gateway_account_id(&self) -> AccountId {
        self.bridge_gateway_account_id
    }
}
