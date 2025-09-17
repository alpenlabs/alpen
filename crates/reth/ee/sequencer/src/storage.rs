use alpen_ee_primitives::EEBlockHash;

use crate::block::{BlockMetadata, BlockPayload};

#[derive(Debug, Clone)]
pub enum StorageError {
    Other(String),
}

pub type ProviderResult<T> = Result<T, StorageError>;

pub trait StorageProvider {
    /// Get latest block in canonical chain
    fn get_latest_ee_block(&self) -> ProviderResult<BlockMetadata>;
    /// Get block in range start..=end
    fn get_ee_block_range(&self, start: u64, end: u64) -> ProviderResult<Vec<BlockMetadata>>;
    /// Get payload to recreate ee block
    /// This is meant for use during consistency checks and recovery of reth db
    fn get_block_payload(&self, blockhash: EEBlockHash) -> ProviderResult<BlockPayload>;

    /// insert block if missing and extend canonical chain tip.
    /// fail if block.parent_blockhash is not current canonical chain tip.
    fn extend_canonical_chain(
        &self,
        package: BlockMetadata,
        block: BlockPayload,
    ) -> ProviderResult<()>;
    /// revert canonical chain to given blockhash.
    /// fail if blockhash is not present in canonical chain.
    fn revert_canonical_chain(&self, to_block: EEBlockHash) -> ProviderResult<()>;

    // TODO
}
