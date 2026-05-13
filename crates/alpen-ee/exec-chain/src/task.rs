use alpen_ee_common::{BlockNumHash, ConsensusHeads, ExecBlockStorage, StorageError};
use strata_acct_types::Hash;
use thiserror::Error;
use tokio::sync::watch;

use crate::state::{ExecChainState, ExecChainStateError};

/// Errors that can occur during execution chain tracker operations.
#[derive(Debug, Error)]
pub(crate) enum ChainTrackerError {
    /// Preconf head channel is closed
    #[error("preconf head channel closed")]
    PreconfChannelClosed,
    /// Block not found in storage
    #[error("missing block: {0:?}")]
    MissingBlock(Hash),
    /// Storage error
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Execution chain state error
    #[error(transparent)]
    ExecChainState(#[from] ExecChainStateError),
}

/// Handles a new block notification by fetching it from storage and appending to chain state.
///
/// Sends a preconf head update if the best tip changes.
pub(crate) async fn handle_new_block<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    hash: Hash,
    storage: &TStorage,
    preconf_tx: &watch::Sender<BlockNumHash>,
) -> Result<(), ChainTrackerError> {
    // Get block from storage
    let record = storage
        .get_exec_block(hash)
        .await?
        .ok_or(ChainTrackerError::MissingBlock(hash))?;

    // Append to tracker state and emit best blocknumhash if changed
    let prev_best = state.tip_blockhash();
    let new_best = state.append_block(record)?;
    if new_best != prev_best {
        preconf_tx
            .send(state.tip_blocknumhash())
            .map_err(|_| ChainTrackerError::PreconfChannelClosed)?;
    }

    Ok(())
}

/// Handles an OL consensus update.
///
/// Updates finalized state if a tracked unfinalized block becomes finalized.
pub(crate) async fn handle_ol_update<TStorage: ExecBlockStorage>(
    state: &mut ExecChainState,
    status: ConsensusHeads,
    storage: &TStorage,
    preconf_tx: &watch::Sender<BlockNumHash>,
) -> Result<(), ChainTrackerError> {
    // we only care about reorgs on the finalized state
    let finalized = *status.finalized();

    if finalized == state.finalized_blockhash() {
        // no need to do anything
        return Ok(());
    }

    if state.contains_unfinalized_block(&finalized) {
        // one of the unfinalized blocks got finalized.
        // update database
        let prev_best = state.tip_blockhash();
        storage.extend_finalized_chain(finalized).await?;

        // update in-memory state
        state
            .prune_finalized(finalized)
            .expect("finalized exists in unfinalized blocks");
        let new_best = state.tip_blockhash();

        if prev_best != new_best {
            // finalization has triggered a reorg of the tip
            preconf_tx
                .send(state.tip_blocknumhash())
                .map_err(|_| ChainTrackerError::PreconfChannelClosed)?;
        }

        return Ok(());
    }

    if state.contains_orphan_block(&finalized) {
        // finalized block is a known but unconnected block
        // TODO: store the finalized state and retry later
        return Ok(());
    }

    // TODO: we have a deep reorg beyond what we consider finalized.
    unimplemented!("deep reorg");
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{
        exec_block_storage_test_fns::create_exec_block, ConsensusHeads, MockExecBlockStorage,
    };
    use strata_acct_types::Hash;

    use super::*;

    fn hash_from_u8(value: u8) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[0] = 1;
        bytes[31] = value;
        Hash::from(bytes)
    }

    #[tokio::test]
    async fn handle_ol_update_uses_multi_block_finalized_extension() {
        let hash0 = hash_from_u8(0);
        let hash1 = hash_from_u8(1);
        let hash2 = hash_from_u8(2);
        let hash3 = hash_from_u8(3);

        let block0 = create_exec_block(0, Hash::default(), hash0, 0);
        let block1 = create_exec_block(1, hash0, hash1, 1);
        let block2 = create_exec_block(2, hash1, hash2, 2);
        let block3 = create_exec_block(3, hash2, hash3, 3);

        let mut state = ExecChainState::new_empty(block0);
        state.append_block(block1).unwrap();
        state.append_block(block2).unwrap();
        state.append_block(block3).unwrap();

        let (preconf_tx, _preconf_rx) = watch::channel(state.tip_blocknumhash());

        let mut storage = MockExecBlockStorage::new();
        storage
            .expect_extend_finalized_chain()
            .withf(move |hash| *hash == hash3)
            .times(1)
            .returning(|_| Ok(()));

        let heads = ConsensusHeads {
            confirmed: hash3,
            confirmed_epoch: 1,
            finalized: hash3,
            finalized_epoch: 1,
        };

        handle_ol_update(&mut state, heads, &storage, &preconf_tx)
            .await
            .unwrap();

        assert_eq!(state.finalized_blockhash(), hash3);
        assert_eq!(state.finalized_blocknum(), 3);
        assert_eq!(state.tip_blockhash(), hash3);
    }
}
