//! Range witness extraction for arbitrary block ranges.
//!
//! Reads per-block accessed-state records produced at production time by
//! [`alpen_reth_exex::AccessedStateGenerator`] (see phase 2 of the EE
//! prover redesign), unions them into a chunk-level accessed-state set,
//! and runs the two pre/post multiproofs. No block re-execution happens
//! here — that work happens once per produced block inside the exex.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use alloy_consensus::{Header, Transaction};
use alloy_primitives::{
    keccak256,
    map::{B256Set, DefaultHashBuilder, HashMap},
    Address, B256, KECCAK256_EMPTY, U256,
};
use alpen_ee_common::AccessedStateStore;
use eyre::{eyre, Result};
use reth_primitives::Block;
use reth_provider::{BlockReader, StateProvider, StateProviderFactory};
use reth_revm::state::Bytecode;
use reth_trie::{HashedPostState, MultiProofTargets, TrieInput};
use reth_trie_common::KeccakKeyHasher;
use rsp_mpt::EthereumState;
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_evm_ee::EvmPartialState;
use tokio::runtime::Handle;
use tracing::debug;

/// Storage key — kept locally; `alpen_reth_exex::StorageKey` is the
/// runtime cache type which we no longer depend on here.
type StorageKey = U256;

/// Witness data extracted for a block range.
#[derive(Debug)]
pub struct RangeWitnessData {
    pub start_block_hash: B256,
    pub end_block_hash: B256,
    /// Serialized `EvmPartialState` (via `strata_codec`).
    pub raw_partial_pre_state: Vec<u8>,
    /// Parent of `start_block` (block before the range). Callers that
    /// need a specific on-wire encoding should encode from here rather
    /// than receiving pre-serialized bytes.
    pub prev_header: Header,
    /// Blocks in range order (start..=end). Available for callers
    /// that need per-block header/body data (e.g. `RawBlockData`
    /// encoding). [`Block`] is reth's type alias for
    /// `alloy_consensus::Block`, specialized to the reth transaction
    /// type.
    pub blocks: Vec<Block>,
}

/// Extracts witness data for block ranges.
///
/// Reads accessed-state per block from [`AccessedStateStore`] (populated
/// by `AccessedStateGenerator` at block-commit time) instead of
/// re-executing blocks. No `EvmConfig` is needed — block execution
/// happens once in the exex, never here.
#[expect(
    missing_debug_implementations,
    reason = "AccessedStateStore trait objects don't impl Debug"
)]
pub struct RangeWitnessExtractor<F, S>
where
    S: AccessedStateStore + 'static,
{
    provider_factory: F,
    accessed_state_store: Arc<S>,
}

impl<F, S> RangeWitnessExtractor<F, S>
where
    F: StateProviderFactory + BlockReader<Block = Block>,
    S: AccessedStateStore + 'static,
{
    pub fn new(provider_factory: F, accessed_state_store: Arc<S>) -> Self {
        Self {
            provider_factory,
            accessed_state_store,
        }
    }

    /// Extracts witness for the block range `[start_block_hash, end_block_hash]` (inclusive).
    pub fn extract_range_witness(
        &self,
        start_block_hash: B256,
        end_block_hash: B256,
    ) -> Result<RangeWitnessData> {
        // Resolve hashes to blocks
        let start_block = self
            .provider_factory
            .block_by_hash(start_block_hash)?
            .ok_or_else(|| eyre!("start block not found for hash {}", start_block_hash))?;
        let end_block = self
            .provider_factory
            .block_by_hash(end_block_hash)?
            .ok_or_else(|| eyre!("end block not found for hash {}", end_block_hash))?;

        let start_block_num = start_block.number;
        let end_block_num = end_block.number;

        if start_block_num > end_block_num {
            return Err(eyre!(
                "invalid block range: start {} > end {}",
                start_block_num,
                end_block_num
            ));
        }

        debug!(start_block_num, end_block_num, %start_block_hash, %end_block_hash, "extracting range witness");

        // Fetch previous block using parent hash
        let prev_block_hash = start_block.header.parent_hash;
        let prev_block = self
            .provider_factory
            .block_by_hash(prev_block_hash)?
            .ok_or_else(|| eyre!("previous block not found for hash {}", prev_block_hash))?;
        let prev_block_num = prev_block.number;
        let start_state_root = prev_block.header.state_root;

        // 1. Execute all blocks to discover accessed state
        let (accessed, blocks) =
            self.read_blocks_and_accessed_state(start_block_num, end_block_num)?;

        // 2. Get providers for pre-range and post-range states
        let pre_state_provider = self
            .provider_factory
            .history_by_block_number(prev_block_num)?;
        let post_state_provider = self
            .provider_factory
            .history_by_block_number(end_block_num)?;

        // 3. Generate multiproofs for all accessed accounts
        let (ethereum_state, bytecodes) = self.build_ethereum_state(
            &pre_state_provider,
            &post_state_provider,
            start_state_root,
            &accessed,
        )?;

        // 4. Get ancestor headers for BLOCKHASH opcode
        let ancestor_headers = self.get_ancestor_headers(start_block_num, &accessed.block_idxs)?;

        // 5. Build and serialize EvmPartialState
        let partial_state = EvmPartialState::new(ethereum_state, bytecodes, ancestor_headers);
        let raw_partial_pre_state = encode_to_vec(&partial_state)
            .map_err(|e| eyre!("failed to encode partial state: {e}"))?;

        Ok(RangeWitnessData {
            start_block_hash,
            end_block_hash,
            raw_partial_pre_state,
            prev_header: prev_block.header,
            blocks,
        })
    }

    /// Read the per-block accessed-state records produced by the
    /// `AccessedStateGenerator` exex for the block range and union them
    /// into a single `AccumulatedState`. Also returns the alloy `Block`
    /// objects (still read from reth, just no execution).
    ///
    /// Bridges async sled reads to the sync caller via
    /// [`Handle::current().block_on`]. The caller must be inside a Tokio
    /// runtime context (e.g. invoked via `task::spawn_blocking` from an
    /// async task) — which the chunk-builder's `seal_batch` is.
    fn read_blocks_and_accessed_state(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> Result<(AccumulatedState, Vec<Block>)> {
        let handle = Handle::try_current()
            .map_err(|e| eyre!("no tokio runtime available for accessed-state reads: {e}"))?;
        let mut acc = AccumulatedState::default();
        let mut blocks = Vec::with_capacity((end_block - start_block + 1) as usize);

        for blk_num in start_block..=end_block {
            let block = self
                .provider_factory
                .block_by_number(blk_num)?
                .ok_or_else(|| eyre!("block {} not found", blk_num))?;
            let block_hash = block.header.hash_slow();
            let block_hash_storage_key = Hash::from(block_hash.0);

            let record = handle
                .block_on(
                    self.accessed_state_store
                        .get_block_accessed_state(block_hash_storage_key),
                )
                .map_err(|e| eyre!("get_block_accessed_state({block_hash:?}): {e}"))?
                .ok_or_else(|| {
                    eyre!(
                        "no accessed-state record for block {} ({block_hash:?}) — exex \
                         AccessedStateGenerator did not run for this block",
                        blk_num,
                    )
                })?;

            // Resolve bytecodes from the content-addressed store and
            // merge into the accumulator.
            for code_hash_bytes in &record.bytecode_hashes {
                let code_hash_storage_key = Hash::from(*code_hash_bytes);
                let code = handle
                    .block_on(
                        self.accessed_state_store
                            .get_bytecode(code_hash_storage_key),
                    )
                    .map_err(|e| eyre!("get_bytecode({code_hash_storage_key:?}): {e}"))?
                    .ok_or_else(|| {
                        eyre!(
                            "no bytecode for code hash {code_hash_storage_key:?} referenced \
                             by block {block_hash:?}",
                        )
                    })?;
                acc.bytecodes
                    .entry(B256::from(*code_hash_bytes))
                    .or_insert_with(|| Bytecode::new_raw(code.into()));
            }
            for stored_account in &record.accounts {
                let address = Address::from(stored_account.address);
                let entry = acc.accounts.entry(address).or_default();
                for slot_bytes in &stored_account.storage_slots {
                    entry.insert(U256::from_be_bytes(*slot_bytes));
                }
            }
            acc.block_idxs
                .extend(record.ancestor_block_numbers.iter().copied());

            // EIP-7702 delegation designations are synthesized from transaction
            // inputs. A designation can be installed and replaced or cleared in
            // the same block, so neither the parent state nor the final execution
            // state is guaranteed to retain its bytecode. The guest still needs
            // that transient code when replaying transactions in order.
            let designation_targets = block
                .body
                .transactions()
                .flat_map(|tx| tx.authorization_list().into_iter().flatten())
                .map(|authorization| *authorization.address());
            let added =
                supplement_authorization_bytecodes(&mut acc.bytecodes, designation_targets)?;
            debug!(
                block_num = blk_num,
                authorization_bytecodes_added = added,
                "closed range witness over EIP-7702 transaction bytecode"
            );

            blocks.push(block);
        }

        Ok((acc, blocks))
    }

    fn build_ethereum_state<P>(
        &self,
        pre_state: &P,
        post_state: &P,
        start_state_root: B256,
        accessed: &AccumulatedState,
    ) -> Result<(EthereumState, BTreeMap<B256, Bytecode>)>
    where
        P: StateProvider,
    {
        // Build touched accounts map: address -> storage keys
        let touched: HashMap<Address, Vec<B256>> = accessed
            .accounts
            .iter()
            .map(|(addr, slots)| {
                let keys = slots
                    .iter()
                    .map(|s| B256::from(s.to_be_bytes::<32>()))
                    .collect();
                (*addr, keys)
            })
            .collect();

        // ALL accessed accounts go into multiproof targets
        let targets = MultiProofTargets::from_iter(touched.iter().map(|(addr, keys)| {
            (
                keccak256(addr),
                B256Set::from_iter(keys.iter().map(keccak256)),
            )
        }));

        // Generate pre-state and post-state multiproofs
        let proof_pre = pre_state.multiproof(
            TrieInput::from_state(HashedPostState::from_bundle_state::<KeccakKeyHasher>([])),
            targets.clone(),
        )?;
        let proof_post = post_state.multiproof(
            TrieInput::from_state(HashedPostState::from_bundle_state::<KeccakKeyHasher>([])),
            targets,
        )?;

        // Extract account proofs
        let mut pre_proofs =
            HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());
        let mut post_proofs =
            HashMap::with_capacity_and_hasher(touched.len(), DefaultHashBuilder::default());

        for (addr, keys) in &touched {
            pre_proofs.insert(*addr, proof_pre.account_proof(*addr, keys)?);
            post_proofs.insert(*addr, proof_post.account_proof(*addr, keys)?);
        }

        let state =
            EthereumState::from_transition_proofs(start_state_root, &pre_proofs, &post_proofs)?;

        // Preserve the code-hash key recorded by AccessedStateGenerator.
        // Re-hashing bytes after they pass through revm representations can produce
        // a different key if legacy-analysis padding was materialized as input.
        // In revm, "legacy" is the normal non-EIP-7702 contract bytecode path.
        // https://github.com/bluealloy/revm/blob/main/crates/bytecode/src/legacy/analysis.rs#L42-L47
        let mut bytecodes: BTreeMap<B256, Bytecode> = accessed
            .bytecodes
            .iter()
            .map(|(hash, bytecode)| (*hash, bytecode.clone()))
            .collect();

        // Close the initial sparse state over bytecode. The transition proof
        // includes every touched account as it existed before the range, so a
        // non-empty code hash in one of those account leaves must be resolvable
        // by WitnessDB even when no individual block record observed a by-hash
        // code fetch. This occurs when a chunk boundary falls between EIP-7702
        // setup and the transaction that consumes or replaces the delegation.
        let pre_state_accounts = pre_proofs.iter().filter_map(|(address, proof)| {
            proof
                .info
                .as_ref()
                .and_then(|account| account.bytecode_hash)
                .map(|code_hash| (*address, code_hash))
        });
        let added =
            supplement_pre_state_bytecodes(&mut bytecodes, pre_state_accounts, |code_hash| {
                Ok(pre_state
                    .bytecode_by_hash(&code_hash)?
                    .map(|bytecode| bytecode.0))
            })?;
        debug!(
            pre_state_bytecodes_added = added,
            total_bytecodes = bytecodes.len(),
            "closed range witness over pre-state account bytecode"
        );

        Ok((state, bytecodes))
    }

    fn get_ancestor_headers(
        &self,
        start_block: u64,
        accessed_idxs: &HashSet<u64>,
    ) -> Result<Vec<Header>> {
        let prev_block = start_block.saturating_sub(1);
        let oldest = accessed_idxs
            .iter()
            .min()
            .copied()
            .unwrap_or(prev_block)
            .min(prev_block);

        (oldest..start_block)
            .rev()
            .map(|n| {
                self.provider_factory
                    .block_by_number(n)?
                    .map(|b| b.header.clone())
                    .ok_or_else(|| eyre!("block {} not found", n))
            })
            .collect()
    }
}

/// Adds every non-reset EIP-7702 delegation designation declared by transactions.
///
/// Authorization validity is deliberately not re-evaluated here. Including an
/// invalid authorization's content-addressed designation is harmless, while
/// trying to duplicate execution's chain-id, signature, nonce, and account-code
/// checks would create a second consensus implementation in the witness builder.
fn supplement_authorization_bytecodes(
    bytecodes: &mut HashMap<B256, Bytecode>,
    targets: impl IntoIterator<Item = Address>,
) -> Result<usize> {
    let mut added = 0;

    for target in targets {
        // A zero target clears delegation code and does not install a
        // designation (EIP-7702 step 8 special case).
        if target.is_zero() {
            continue;
        }

        let bytecode = Bytecode::new_eip7702(target);
        let code_hash = bytecode.hash_slow();
        if let Some(existing) = bytecodes.get(&code_hash) {
            if existing.original_bytes() != bytecode.original_bytes() {
                return Err(eyre!(
                    "conflicting EIP-7702 designation bytecode for hash {code_hash}: target={target}"
                ));
            }
            continue;
        }

        bytecodes.insert(code_hash, bytecode);
        added += 1;
    }

    Ok(added)
}

/// Adds bytecode referenced by accessed accounts in the range's initial state.
///
/// Per-block access records are necessary but not sufficient for a multi-block
/// witness: they describe execution-time fetches, while the sparse pre-state
/// independently contains account leaves with code hashes. Including the code
/// behind every non-empty pre-state hash makes that witness self-contained.
fn supplement_pre_state_bytecodes(
    bytecodes: &mut BTreeMap<B256, Bytecode>,
    accounts: impl IntoIterator<Item = (Address, B256)>,
    mut bytecode_by_hash: impl FnMut(B256) -> Result<Option<Bytecode>>,
) -> Result<usize> {
    let mut added = 0;

    for (address, code_hash) in accounts {
        if code_hash == KECCAK256_EMPTY {
            continue;
        }

        if let Some(existing) = bytecodes.get(&code_hash) {
            validate_bytecode_hash(existing, code_hash, address)?;
            continue;
        }

        let bytecode = bytecode_by_hash(code_hash)?.ok_or_else(|| {
            eyre!("pre-state bytecode {code_hash} for accessed account {address} was not found")
        })?;
        validate_bytecode_hash(&bytecode, code_hash, address)?;
        bytecodes.insert(code_hash, bytecode);
        added += 1;
    }

    Ok(added)
}

fn validate_bytecode_hash(bytecode: &Bytecode, expected: B256, address: Address) -> Result<()> {
    let actual = keccak256(bytecode.original_bytes());
    if actual != expected {
        return Err(eyre!(
            "pre-state bytecode hash mismatch for accessed account {address}: \
             expected={expected}, actual={actual}"
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct AccumulatedState {
    accounts: HashMap<Address, HashSet<StorageKey>>,
    bytecodes: HashMap<B256, Bytecode>,
    block_idxs: HashSet<u64>,
}

#[cfg(test)]
mod tests {
    use std::iter::{empty, once};

    use alloy_primitives::Bytes;

    use super::*;

    #[test]
    fn supplements_transient_eip7702_authorization_bytecode() {
        let target = Address::repeat_byte(0x24);
        let expected = Bytecode::new_eip7702(target);
        let code_hash = expected.hash_slow();
        let mut bytecodes = HashMap::default();

        let added =
            supplement_authorization_bytecodes(&mut bytecodes, [target, Address::ZERO, target])
                .unwrap();

        assert_eq!(added, 1);
        assert_eq!(bytecodes.len(), 1);
        assert_eq!(bytecodes.get(&code_hash), Some(&expected));
    }

    #[test]
    fn rejects_conflicting_authorization_bytecode() {
        let target = Address::repeat_byte(0x24);
        let code_hash = Bytecode::new_eip7702(target).hash_slow();
        let mut bytecodes = HashMap::from_iter([(
            code_hash,
            Bytecode::new_raw(Bytes::from_static(&[0x60, 0x00])),
        )]);

        let err = supplement_authorization_bytecodes(&mut bytecodes, once(target)).unwrap_err();

        assert!(err
            .to_string()
            .contains("conflicting EIP-7702 designation bytecode"));
    }

    #[test]
    fn empty_authorization_targets_do_not_change_bytecodes() {
        let mut bytecodes = HashMap::default();

        let added = supplement_authorization_bytecodes(&mut bytecodes, empty()).unwrap();

        assert_eq!(added, 0);
        assert!(bytecodes.is_empty());
    }

    #[test]
    fn supplements_delegation_code_referenced_only_by_pre_state_account() {
        let account = Address::repeat_byte(0x42);
        let bytecode = Bytecode::new_eip7702(Address::repeat_byte(0x24));
        let code_hash = bytecode.hash_slow();
        let mut bytecodes = BTreeMap::new();

        let added = supplement_pre_state_bytecodes(
            &mut bytecodes,
            once((account, code_hash)),
            |requested| {
                assert_eq!(requested, code_hash);
                Ok(Some(bytecode.clone()))
            },
        )
        .unwrap();

        assert_eq!(added, 1);
        assert_eq!(bytecodes.get(&code_hash), Some(&bytecode));
    }

    #[test]
    fn does_not_refetch_bytecode_already_captured_by_block_record() {
        let account = Address::repeat_byte(0x42);
        let bytecode = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x2a, 0x00]));
        let code_hash = bytecode.hash_slow();
        let mut bytecodes = BTreeMap::from([(code_hash, bytecode)]);

        let added = supplement_pre_state_bytecodes(
            &mut bytecodes,
            once((account, code_hash)),
            |_| -> Result<Option<Bytecode>> {
                panic!("existing bytecode must not be fetched again")
            },
        )
        .unwrap();

        assert_eq!(added, 0);
        assert_eq!(bytecodes.len(), 1);
    }

    #[test]
    fn reports_missing_pre_state_bytecode_before_guest_execution() {
        let account = Address::repeat_byte(0x42);
        let code_hash = B256::repeat_byte(0x57);
        let mut bytecodes = BTreeMap::new();

        let err =
            supplement_pre_state_bytecodes(&mut bytecodes, once((account, code_hash)), |_| {
                Ok(None)
            })
            .unwrap_err();

        assert!(err.to_string().contains("pre-state bytecode"));
        assert!(err.to_string().contains(&code_hash.to_string()));
        assert!(err.to_string().contains(&account.to_string()));
    }
}
