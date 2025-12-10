use std::num::NonZeroU8;

use strata_acct_types::AccountId;

/// Default target blocktime in millis
const DEFAULT_BLOCKTIME_MS: u64 = 1000;
/// Default number of deposits to process per ee block.
const DEFAULT_DEPOSITS_PER_BLOCK: NonZeroU8 = NonZeroU8::new(16).unwrap();
/// Default bridge gateway account on OL.
const DEFAULT_BRIDGE_GATEWAY_ACCOUNT: AccountId = AccountId::special(1); // TODO: correct account id
/// Default time to wait on errors during block building.
const DEFAULT_ERROR_BACKOFF_MS: u64 = 100;

#[derive(Debug)]
pub struct BlockBuilderConfig {
    /// Target blocktime in ms
    blocktime_ms: u64,
    /// Max number of deposits that can be processed in a single EE block.
    max_deposits_per_block: NonZeroU8,
    /// [`AccountId`] of bridge gateway on OL.
    bridge_gateway_account_id: AccountId,
    /// Base backoff time to delay on errors during block building.
    error_backoff_ms: u64,
}

impl Default for BlockBuilderConfig {
    fn default() -> Self {
        Self {
            blocktime_ms: DEFAULT_BLOCKTIME_MS,
            max_deposits_per_block: DEFAULT_DEPOSITS_PER_BLOCK,
            bridge_gateway_account_id: DEFAULT_BRIDGE_GATEWAY_ACCOUNT,
            error_backoff_ms: DEFAULT_ERROR_BACKOFF_MS,
        }
    }
}

impl BlockBuilderConfig {
    pub fn with_blocktime_ms(mut self, blocktime_ms: u64) -> Self {
        self.blocktime_ms = blocktime_ms;
        self
    }

    pub fn with_max_deposits_per_block(
        mut self,
        max_deposits_per_block: impl Into<NonZeroU8>,
    ) -> Self {
        self.max_deposits_per_block = max_deposits_per_block.into();
        self
    }

    pub fn with_bridge_gateway_account_id(mut self, bridge_gateway_account_id: AccountId) -> Self {
        self.bridge_gateway_account_id = bridge_gateway_account_id;
        self
    }

    pub fn with_error_backoff_ms(mut self, error_backoff_ms: u64) -> Self {
        self.error_backoff_ms = error_backoff_ms;
        self
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

    pub fn error_backoff_ms(&self) -> u64 {
        self.error_backoff_ms
    }
}
