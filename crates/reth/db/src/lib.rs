//! Database for Reth.

#[cfg(feature = "rocksdb")]
pub mod rocksdb;
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
pub mod sled;

#[cfg(feature = "rocksdb")]
#[allow(unused_extern_crates)]
extern crate rockbound as _;
#[cfg(all(feature = "sled", not(feature = "rocksdb")))]
#[allow(unused_extern_crates)]
extern crate sled as _;

// Consume sled-related dependencies when rocksdb is active to avoid unused crate warnings
#[cfg(feature = "rocksdb")]
use strata_db_store_sled as _;
#[cfg(feature = "rocksdb")]
#[allow(unused_extern_crates)]
extern crate sled as _;
#[cfg(feature = "rocksdb")]
#[allow(unused_extern_crates)]
extern crate typed_sled as _;

// Consume dev dependencies to avoid unused warnings in tests
use alpen_reth_statediff::BlockStateDiff;
use revm_primitives::alloy_primitives::B256;
#[cfg(test)]
use serde as _;
#[cfg(test)]
use serde_json as _;
pub use strata_db::{errors, DbResult};
use strata_proofimpl_evm_ee_stf::EvmBlockStfInput;

pub trait WitnessStore {
    fn put_block_witness(&self, block_hash: B256, witness: &EvmBlockStfInput) -> DbResult<()>;
    fn del_block_witness(&self, block_hash: B256) -> DbResult<()>;
}

pub trait WitnessProvider {
    fn get_block_witness(&self, block_hash: B256) -> DbResult<Option<EvmBlockStfInput>>;
    fn get_block_witness_raw(&self, block_hash: B256) -> DbResult<Option<Vec<u8>>>;
}

pub trait StateDiffStore {
    fn put_state_diff(
        &self,
        block_hash: B256,
        block_number: u64,
        state_diff: &BlockStateDiff,
    ) -> DbResult<()>;
    fn del_state_diff(&self, block_hash: B256) -> DbResult<()>;
}

pub trait StateDiffProvider {
    fn get_state_diff_by_hash(&self, block_hash: B256) -> DbResult<Option<BlockStateDiff>>;
    fn get_state_diff_by_number(&self, block_number: u64) -> DbResult<Option<BlockStateDiff>>;
}
