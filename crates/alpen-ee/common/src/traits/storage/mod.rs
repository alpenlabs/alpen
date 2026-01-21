mod account;
mod batch;
mod errors;
mod exec_block;
mod utils;

#[cfg(feature = "test-utils")]
pub use account::{tests, MockStorage};
pub use account::{OLBlockOrEpoch, Storage};
pub use batch::BatchStorage;
#[cfg(feature = "test-utils")]
pub use batch::{tests as batch_storage_test_fns, MockBatchStorage};
pub use errors::StorageError;
pub use exec_block::ExecBlockStorage;
#[cfg(feature = "test-utils")]
pub use exec_block::{exec_block_storage_test_fns, MockExecBlockStorage};
pub use utils::{
    require_best_ee_account_state, require_best_finalized_block, require_latest_batch,
};
