//! EVM bytecode-dedup layer of the DA witness: determining which account
//! bytecodes the published blob omits (because a prior batch published them),
//! resolving their preimages, and the [`DedupWitnessResolver`] seam that turns a
//! published blob into a [`DedupWitness`].

use std::collections::BTreeSet;

use alloy_primitives::{keccak256, B256};
use alpen_ee_common::AccessedStateStore;
use alpen_ee_da_types::{BytecodePreimage, DaBlob, DedupWitness};
use alpen_reth_db::StateDiffProvider;
use alpen_reth_statediff::{AccountChange, BatchBuilder, BatchStateDiff};
use async_trait::async_trait;
use strata_acct_types::Hash;

use super::DaWitnessBuildError;

/// Resolves the supplementary [`DedupWitness`] the guest needs to verify a
/// published DA blob whose references aren't all carried inline.
///
/// Implemented host-side by a type holding the providers (state-diff provider +
/// bytecode store). Today only bytecode preimages are resolved; future DA-dedup
/// optimizations (e.g. account/storage serials) add per-kind resolution here,
/// each returning the resolved value plus a membership proof.
#[async_trait]
pub trait DedupWitnessResolver {
    /// Resolves preimages for bytecodes the blob references but omits (DA dedup).
    async fn resolve_bytecode_preimages(
        &self,
        blob: &DaBlob,
        batch_block_hashes: &[B256],
    ) -> Result<Vec<BytecodePreimage>, DaWitnessBuildError>;

    /// Produces the full [`DedupWitness`] — all resolved supplementary data (and,
    /// in future, their membership proofs). Future per-kind resolvers extend this.
    async fn resolve_dedup_witness(
        &self,
        blob: &DaBlob,
        batch_block_hashes: &[B256],
    ) -> Result<DedupWitness, DaWitnessBuildError> {
        Ok(DedupWitness::new(
            self.resolve_bytecode_preimages(blob, batch_block_hashes)
                .await?,
        ))
    }
}

/// Host-side [`DedupWitnessResolver`] backed by the node's Reth state-diff
/// provider and bytecode store.
///
/// The EVM-specific interpretation of the published blob (which bytecodes it
/// omitted, and where to fetch their preimages) lives here. Generic over the
/// provider/store types so it carries no concrete node dependency.
#[derive(Debug)]
pub struct DaDedupResolver<'a, D: ?Sized, B> {
    state_diff_provider: &'a D,
    bytecode_store: &'a B,
}

impl<'a, D: ?Sized, B> DaDedupResolver<'a, D, B> {
    pub fn new(state_diff_provider: &'a D, bytecode_store: &'a B) -> Self {
        Self {
            state_diff_provider,
            bytecode_store,
        }
    }
}

#[async_trait]
impl<D, B> DedupWitnessResolver for DaDedupResolver<'_, D, B>
where
    D: StateDiffProvider + ?Sized,
    B: AccessedStateStore,
{
    async fn resolve_bytecode_preimages(
        &self,
        blob: &DaBlob,
        batch_block_hashes: &[B256],
    ) -> Result<Vec<BytecodePreimage>, DaWitnessBuildError> {
        // Primary source: the full batch diff (host data from before the
        // publication filter ran).
        let batch_diff = build_batch_state_diff(batch_block_hashes, self.state_diff_provider)?;
        let (mut preimages, unresolved) = bytecode_preimages_from_batch_diff(blob, &batch_diff);

        // Secondary source: the node bytecode store, for bytecodes deployed in a
        // prior batch and so absent from this batch's diff.
        for code_hash in unresolved {
            let storage_key = Hash::from(code_hash.0);
            let bytecode = self
                .bytecode_store
                .get_bytecode(storage_key)
                .await
                .map_err(|e| DaWitnessBuildError::BytecodeStore {
                    hash: format!("{storage_key:?}"),
                    error: e.to_string(),
                })?
                .ok_or_else(|| DaWitnessBuildError::BytecodeMissing(format!("{storage_key:?}")))?;
            preimages.push(BytecodePreimage::new(bytecode));
        }

        Ok(preimages)
    }
}

/// Returns account code hashes referenced by the blob but absent from the current blob bytecodes.
///
/// These are the hashes affected by DA bytecode dedup: the account diff still
/// advertises a `code_hash`, but the current L1 blob no longer carries the
/// matching bytecode bytes.
fn deduped_account_code_hashes(blob: &DaBlob) -> Vec<B256> {
    let empty_code_hash = keccak256([]);
    let mut missing = BTreeSet::new();

    for change in blob.state_diff.accounts.values() {
        let account_diff = match change {
            AccountChange::Created(diff) | AccountChange::Updated(diff) => diff,
            AccountChange::Deleted => continue,
        };
        let Some(code_hash) = account_diff.code_hash.new_value().map(|hash| hash.0) else {
            continue;
        };
        if code_hash == empty_code_hash
            || blob.state_diff.deployed_bytecodes.contains_key(&code_hash)
        {
            continue;
        }
        missing.insert(code_hash);
    }

    missing.into_iter().collect()
}

/// Resolves deduped-bytecode preimages from the batch state diff.
///
/// The DA blob handed to the guest has already passed the publication filter, so
/// bytecodes published by earlier batches can be missing from
/// `blob.state_diff.deployed_bytecodes`. The batch's full per-block state diff is
/// the host's local copy of the same executed batch *before* that filter ran, and
/// still carries deployment bytecodes the blob omitted — so it is the primary
/// source for those preimages. Using it avoids depending on the accessed-state
/// cache, which only holds bytecode loaded via `code_by_hash` and can miss a
/// contract that was deployed but never read again.
///
/// Returns the preimages resolved from `batch_state_diff`, plus the code hashes
/// *not* found there, which the caller must resolve from a secondary source
/// (e.g. the node bytecode store).
///
/// NOTE: these preimages prove bytecode identity, not prior L1 publication.
/// TODO(STR-1907): replace with an authenticated prior-publication proof.
fn bytecode_preimages_from_batch_diff(
    blob: &DaBlob,
    batch_state_diff: &BatchStateDiff,
) -> (Vec<BytecodePreimage>, Vec<B256>) {
    let mut preimages = Vec::new();
    let mut unresolved = Vec::new();

    for code_hash in deduped_account_code_hashes(blob) {
        match batch_state_diff.deployed_bytecodes.get(&code_hash) {
            Some(bytecode) => preimages.push(BytecodePreimage::new(bytecode.to_vec())),
            None => unresolved.push(code_hash),
        }
    }

    (preimages, unresolved)
}

/// Aggregates the per-block state diffs for a batch into one [`BatchStateDiff`].
///
/// This is the host's full pre-publication-filter view: the DA blob handed to the
/// guest has already had DA dedup applied, but these per-block diffs still carry
/// the bytecodes it omitted. No persisted batch diff exists, so it is rebuilt
/// block by block — the same way the publish path aggregates it.
fn build_batch_state_diff(
    block_hashes: &[B256],
    state_diff_provider: &(impl StateDiffProvider + ?Sized),
) -> Result<BatchStateDiff, DaWitnessBuildError> {
    let mut builder = BatchBuilder::new();

    for b256 in block_hashes {
        let block_diff = state_diff_provider
            .get_state_diff_by_hash(*b256)
            .map_err(|e| DaWitnessBuildError::StateDiffProvider {
                block: format!("{b256:?}"),
                error: e.to_string(),
            })?
            .ok_or_else(|| DaWitnessBuildError::StateDiffMissing(format!("{b256:?}")))?;
        builder.apply_block(&block_diff);
    }

    Ok(builder.build())
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, Bytes, U256};
    use alpen_ee_da_types::EvmHeaderSummary;
    use alpen_reth_statediff::AccountDiff;

    use super::*;

    #[test]
    fn bytecode_preimages_from_batch_diff_recovers_deduped_deployment_bytecode() {
        let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]);
        let code_hash = keccak256(bytecode.as_ref());
        let address = Address::from([0x11; 20]);

        let mut filtered_diff = BatchStateDiff::new();
        filtered_diff.accounts.insert(
            address,
            AccountChange::Created(AccountDiff::new_created(U256::ZERO, 1, code_hash)),
        );

        let mut batch_diff = filtered_diff.clone();
        batch_diff
            .deployed_bytecodes
            .insert(code_hash, bytecode.clone());

        let blob = DaBlob {
            update_seq_no: 7,
            evm_header: EvmHeaderSummary {
                block_num: 10,
                timestamp: 1_700_000_000,
                base_fee: 100,
                gas_used: 21_000,
                gas_limit: 36_000_000,
            },
            state_diff: filtered_diff,
        };

        let (preimages, unresolved) = bytecode_preimages_from_batch_diff(&blob, &batch_diff);

        assert!(unresolved.is_empty());
        assert_eq!(preimages.len(), 1);
        assert_eq!(preimages[0].bytecode(), bytecode.as_ref());
    }
}
