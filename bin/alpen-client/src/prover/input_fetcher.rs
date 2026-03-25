//! Input fetchers for EE proof tasks.
//!
//! Stub implementations pending PR #1522 which introduces `EeChunkProofInput`
//! and `EeAcctProofInput` types from `proof-impl/alpen-chunk` and `proof-impl/alpen-acct`.

use std::sync::Arc;

use alpen_ee_common::{BatchStorage, EeProofTask, ExecBlockStorage};
use async_trait::async_trait;
use strata_paas::InputFetcher;

/// Fetches inputs for chunk proof generation.
///
/// Given `EeProofTask::Chunk { batch_id, chunk_idx }`, constructs the proof input by:
/// 1. Fetching the `Batch` from storage to get block hashes for this chunk.
/// 2. Fetching `ExecBlockRecord` for each block in the chunk.
/// 3. Fetching partial pre-state witness from Reth.
/// 4. Building `EeChunkProofInput` with genesis, `PrivateInput`.
///
/// TODO(PR #1522): Replace stub with real implementation using `EeChunkProofInput`.
#[derive(Clone)]
pub(crate) struct ChunkInputFetcher {
    _batch_storage: Arc<dyn BatchStorage>,
    _block_storage: Arc<dyn ExecBlockStorage>,
}

impl ChunkInputFetcher {
    pub(crate) fn new(
        batch_storage: Arc<dyn BatchStorage>,
        block_storage: Arc<dyn ExecBlockStorage>,
    ) -> Self {
        Self {
            _batch_storage: batch_storage,
            _block_storage: block_storage,
        }
    }
}

/// Fetches inputs for account proof generation.
///
/// Given `EeProofTask::Acct { batch_id }`, constructs the proof input by:
/// 1. Fetching completed chunk proofs from the proof store.
/// 2. Building `EeAcctProofInput` with genesis, chunk predicate key, chunk proofs.
///
/// TODO(PR #1522): Replace stub with real implementation using `EeAcctProofInput`.
#[derive(Clone)]
pub(crate) struct AcctInputFetcher;

impl AcctInputFetcher {
    pub(crate) fn new() -> Self {
        Self
    }
}

/// Placeholder input type until PR #1522 provides real proof input types.
///
/// Will be replaced by `EeChunkProofInput` / `EeAcctProofInput`.
pub(crate) struct StubProofInput;

/// Error type for input fetching.
#[derive(Debug)]
pub(crate) struct InputFetchError(String);

impl std::fmt::Display for InputFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InputFetchError {}

#[async_trait]
impl InputFetcher<EeProofTask> for ChunkInputFetcher {
    type Input = StubProofInput;
    type Error = InputFetchError;

    async fn fetch_input(&self, _program: &EeProofTask) -> Result<Self::Input, Self::Error> {
        // TODO(PR #1522): Implement real chunk input fetching.
        // 1. Extract batch_id, chunk_idx from program
        // 2. Fetch Batch from batch_storage
        // 3. Slice blocks for this chunk
        // 4. For each block, fetch ExecBlockRecord
        // 5. Fetch partial pre-state from Reth
        // 6. Build PrivateInput -> EeChunkProofInput
        todo!("chunk input fetching blocked on PR #1522 (alpen-chunk proof-impl)")
    }
}

#[async_trait]
impl InputFetcher<EeProofTask> for AcctInputFetcher {
    type Input = StubProofInput;
    type Error = InputFetchError;

    async fn fetch_input(&self, _program: &EeProofTask) -> Result<Self::Input, Self::Error> {
        // TODO(PR #1522): Implement real acct input fetching.
        // 1. Extract batch_id from program
        // 2. Fetch chunk proof receipts from proof store
        // 3. Build ChunkInput (transition + proof bytes) for each chunk
        // 4. Build EePrivateInput + update manifest
        // 5. Return EeAcctProofInput { genesis, predicate_key, ee_input, update_input }
        todo!("acct input fetching blocked on PR #1522 (alpen-acct proof-impl)")
    }
}
