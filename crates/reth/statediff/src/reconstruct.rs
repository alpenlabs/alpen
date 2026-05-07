//! State reconstruction from batch diffs.

#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::HashMap;

use alpen_chainspec::chain_value_parser;
use revm_primitives::{alloy_primitives::Address, B256, U256};
use rsp_mpt::EthereumState;
use strata_da_framework::ContextlessDaWrite;
use strata_mpt::{keccak, MptNode, StateAccount, EMPTY_ROOT, KECCAK_EMPTY};
use thiserror::Error as ThisError;

use crate::{
    batch::{AccountChange, BatchStateDiff},
    block::AccountSnapshot,
};

/// Error that may occur during state reconstruction.
#[derive(Debug, ThisError)]
pub enum ReconstructError {
    #[error("MPT: {0}")]
    Mpt(#[from] strata_mpt::Error),
    #[error("DA apply: {0}")]
    Da(#[from] strata_da_framework::DaError),
}

/// Reconstructs EVM state by applying [`BatchStateDiff`]s sequentially.
///
/// Used primarily for testing to verify that state roots reconstructed
/// from diffs match the actual state roots from EE blocks.
#[derive(Clone, Default, Debug)]
pub struct StateReconstructor {
    state_trie: MptNode,
    storage_trie: HashMap<Address, MptNode>,
}

impl StateReconstructor {
    /// Creates a new empty reconstructor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a reconstructor initialized with genesis state from a chain spec.
    pub fn from_chain_spec(spec: &str) -> Result<Self, eyre::Error> {
        let chain_spec = chain_value_parser(spec)?;

        let mut reconstructor = Self::new();
        for (address, account) in chain_spec.genesis.alloc.iter() {
            let mut state_account = StateAccount {
                nonce: account.nonce.unwrap_or(0),
                balance: account.balance,
                storage_root: EMPTY_ROOT,
                code_hash: account
                    .code
                    .as_ref()
                    .map(|bytes| keccak(bytes).into())
                    .unwrap_or(KECCAK_EMPTY),
            };

            if let Some(slots) = &account.storage {
                if !slots.is_empty() {
                    let acc_storage_trie = reconstructor.storage_trie.entry(*address).or_default();
                    for (slot_key, slot_value) in slots.iter() {
                        if slot_value != &B256::ZERO {
                            acc_storage_trie.insert_rlp(&keccak(slot_key), *slot_value)?;
                        }
                    }
                    state_account.storage_root = acc_storage_trie.hash();
                }
            }

            reconstructor
                .state_trie
                .insert_rlp(&keccak(address), state_account)?;
        }

        Ok(reconstructor)
    }

    /// Applies a [`BatchStateDiff`] to the current state.
    pub fn apply_diff(&mut self, diff: &BatchStateDiff) -> Result<(), ReconstructError> {
        for (address, change) in &diff.accounts {
            let acc_info_trie_path = keccak(address);

            match change {
                AccountChange::Created(account_diff) | AccountChange::Updated(account_diff) => {
                    // Get current account state (if exists)
                    let current: Option<StateAccount> = self
                        .state_trie
                        .get_rlp(&acc_info_trie_path)
                        .unwrap_or_default();

                    // Build snapshot from current state and apply diff
                    let mut snapshot = current
                        .as_ref()
                        .map(AccountSnapshot::from)
                        .unwrap_or_default();

                    account_diff.apply(&mut snapshot)?;

                    let mut state_account = StateAccount {
                        nonce: snapshot.nonce,
                        balance: snapshot.balance,
                        storage_root: Default::default(),
                        code_hash: snapshot.code_hash,
                    };

                    // Skip empty accounts
                    if state_account.is_account_empty() {
                        continue;
                    }

                    // Calculate storage root
                    state_account.storage_root = {
                        let acc_storage_trie = self.storage_trie.entry(*address).or_default();
                        if let Some(storage_diff) = diff.storage.get(address) {
                            for (slot_key, slot_value) in storage_diff.iter() {
                                let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                                match slot_value {
                                    Some(v) if !v.is_zero() => {
                                        acc_storage_trie.insert_rlp(&slot_trie_path, *v)?;
                                    }
                                    _ => {
                                        acc_storage_trie.delete(&slot_trie_path)?;
                                    }
                                }
                            }
                        }
                        acc_storage_trie.hash()
                    };

                    self.state_trie
                        .insert_rlp(&acc_info_trie_path, state_account)?;
                }
                AccountChange::Deleted => {
                    self.state_trie.delete(&acc_info_trie_path)?;
                    self.storage_trie.remove(address);
                }
            }
        }

        // Handle storage changes for accounts not in accounts map
        // (e.g., storage-only changes)
        for (address, storage_diff) in &diff.storage {
            if diff.accounts.contains_key(address) {
                continue; // Already handled above
            }

            let acc_info_trie_path = keccak(address);
            let current: Option<StateAccount> = self
                .state_trie
                .get_rlp(&acc_info_trie_path)
                .unwrap_or_default();

            if let Some(mut state_account) = current {
                let acc_storage_trie = self.storage_trie.entry(*address).or_default();
                for (slot_key, slot_value) in storage_diff.iter() {
                    let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                    match slot_value {
                        Some(v) if !v.is_zero() => {
                            acc_storage_trie.insert_rlp(&slot_trie_path, *v)?;
                        }
                        _ => {
                            acc_storage_trie.delete(&slot_trie_path)?;
                        }
                    }
                }
                state_account.storage_root = acc_storage_trie.hash();
                self.state_trie
                    .insert_rlp(&acc_info_trie_path, state_account)?;
            }
        }

        Ok(())
    }

    /// Returns the current state root.
    pub fn state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Returns the current storage root for an account.
    pub fn storage_root(&self, address: Address) -> B256 {
        self.storage_trie
            .get(&address)
            .map(|t| t.hash())
            .unwrap_or(EMPTY_ROOT)
    }

    /// Returns the value at a storage slot.
    pub fn storage_slot(&self, address: Address, slot_key: U256) -> U256 {
        self.storage_trie
            .get(&address)
            .unwrap_or(&MptNode::default())
            .get_rlp::<U256>(&keccak(slot_key.to_be_bytes::<32>()))
            .unwrap_or_default()
            .unwrap_or_default()
    }

    /// Returns the account state.
    pub fn account(&self, address: Address) -> Option<StateAccount> {
        self.state_trie
            .get_rlp(&keccak(address))
            .unwrap_or_default()
    }

    /// Creates a reconstructor from explicit canonical account and storage state.
    ///
    /// This helper exists for oracle tests that need to seed pre-state directly
    /// from test fixtures instead of going through a chain spec or DB-backed
    /// state source.
    ///
    /// Empty accounts are skipped during seeding, matching the canonical-state
    /// oracle behavior used by the reconstruction tests.
    #[cfg(test)]
    pub(crate) fn from_state_parts(
        accounts: &BTreeMap<Address, StateAccount>,
        storage: &BTreeMap<Address, BTreeMap<U256, U256>>,
    ) -> Result<Self, ReconstructError> {
        let mut reconstructor = Self::new();

        for (address, account) in accounts {
            let mut state_account = account.clone();
            if state_account.is_account_empty() {
                continue;
            }

            let mut storage_trie = MptNode::default();

            if let Some(account_storage) = storage.get(address) {
                for (slot_key, slot_value) in account_storage {
                    if slot_value.is_zero() {
                        continue;
                    }

                    storage_trie.insert_rlp(&keccak(slot_key.to_be_bytes::<32>()), *slot_value)?;
                }
            }

            state_account.storage_root = storage_trie.hash();
            if !storage_trie.is_empty() {
                reconstructor.storage_trie.insert(*address, storage_trie);
            }

            reconstructor
                .state_trie
                .insert_rlp(&keccak(address), state_account)?;
        }

        Ok(reconstructor)
    }
}

/// Applies a [`BatchStateDiff`] to a populated [`EthereumState`] sparse-MPT
/// witness in place.
///
/// Mirrors [`StateReconstructor::apply_diff`] but operates on the sparse
/// MPT shape produced by `RangeWitnessExtractor` (and consumed by the
/// chunk pipeline), so the EE outer (acct) proof can apply a reassembled
/// diff to the same pre-state the chunk witnesses already carry and read
/// the resulting state root via [`EthereumState::state_root`].
///
/// `EthereumState` keys both `state_trie` and `storage_tries` by hashed
/// address (`B256 = keccak256(Address)`); the lookup translation is done
/// inline via [`keccak`].
///
/// MPT primitive failures (malformed sparse trie, missing nodes, …) are
/// treated as panics — they only happen on a broken witness, which is a
/// host bug we don't try to handle in-proof. Matches the pattern used by
/// [`EthereumState::update`] itself.
pub fn apply_batch_state_diff_to_ethereum_state(
    state: &mut EthereumState,
    diff: &BatchStateDiff,
) -> Result<(), ReconstructError> {
    for (address, change) in &diff.accounts {
        let hashed_addr: B256 = keccak(address).into();

        match change {
            AccountChange::Created(account_diff) | AccountChange::Updated(account_diff) => {
                let current: Option<StateAccount> = state
                    .state_trie
                    .get_rlp(hashed_addr.as_slice())
                    .unwrap_or_default();

                let mut snapshot = current
                    .as_ref()
                    .map(AccountSnapshot::from)
                    .unwrap_or_default();

                account_diff.apply(&mut snapshot)?;

                let mut state_account = StateAccount {
                    nonce: snapshot.nonce,
                    balance: snapshot.balance,
                    storage_root: Default::default(),
                    code_hash: snapshot.code_hash,
                };

                if state_account.is_account_empty() {
                    continue;
                }

                state_account.storage_root = {
                    let acc_storage_trie = state.storage_tries.entry(hashed_addr).or_default();
                    if let Some(storage_diff) = diff.storage.get(address) {
                        for (slot_key, slot_value) in storage_diff.iter() {
                            let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                            match slot_value {
                                Some(v) if !v.is_zero() => {
                                    acc_storage_trie
                                        .insert_rlp(&slot_trie_path, *v)
                                        .expect("storage trie insert");
                                }
                                _ => {
                                    acc_storage_trie
                                        .delete(&slot_trie_path)
                                        .expect("storage trie delete");
                                }
                            }
                        }
                    }
                    acc_storage_trie.hash()
                };

                state
                    .state_trie
                    .insert_rlp(hashed_addr.as_slice(), state_account)
                    .expect("state trie insert");
            }
            AccountChange::Deleted => {
                state
                    .state_trie
                    .delete(hashed_addr.as_slice())
                    .expect("state trie delete");
                state.storage_tries.remove(&hashed_addr);
            }
        }
    }

    // Storage-only changes for accounts not in the accounts map (e.g. an
    // existing account whose nonce/balance/code didn't change but whose
    // storage did).
    for (address, storage_diff) in &diff.storage {
        if diff.accounts.contains_key(address) {
            continue;
        }

        let hashed_addr: B256 = keccak(address).into();
        let current: Option<StateAccount> = state
            .state_trie
            .get_rlp(hashed_addr.as_slice())
            .unwrap_or_default();

        if let Some(mut state_account) = current {
            let acc_storage_trie = state.storage_tries.entry(hashed_addr).or_default();
            for (slot_key, slot_value) in storage_diff.iter() {
                let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                match slot_value {
                    Some(v) if !v.is_zero() => {
                        acc_storage_trie
                            .insert_rlp(&slot_trie_path, *v)
                            .expect("storage-only insert");
                    }
                    _ => {
                        acc_storage_trie
                            .delete(&slot_trie_path)
                            .expect("storage-only delete");
                    }
                }
            }
            state_account.storage_root = acc_storage_trie.hash();
            state
                .state_trie
                .insert_rlp(hashed_addr.as_slice(), state_account)
                .expect("state trie insert (storage-only)");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_mpt::EMPTY_ROOT;

    use super::*;
    use crate::{
        test_utils::{
            account_change, addr, batch_diff, block_diff, bytecode, canonical_accounts,
            canonical_state_root, deployed_bytecode, hash, slot, snapshot, state_account,
            storage_change, value, CanonicalState,
        },
        BlockStateChanges,
    };

    // The oracle below intentionally shares the same MPT primitives as the
    // reconstructor. These tests verify diff application produces the expected
    // post-state inputs and roots, not that the root algorithm is independently
    // reimplemented.
    fn assert_reconstruction_matches(
        reconstructor: &StateReconstructor,
        expected_state: &CanonicalState,
        expected_slots: &[(Address, U256)],
        expected_bytecodes: &[(B256, &[u8])],
        diff: &BatchStateDiff,
    ) {
        let expected_accounts = canonical_accounts(expected_state).unwrap();
        assert_eq!(
            reconstructor.state_root(),
            canonical_state_root(expected_state).unwrap()
        );

        let addresses = expected_state
            .accounts
            .keys()
            .chain(expected_state.storage.keys())
            .copied()
            .collect::<BTreeSet<_>>();

        for address in addresses {
            let actual_account = reconstructor.account(address);
            let expected_account = expected_accounts.get(&address);

            match (actual_account, expected_account) {
                (Some(actual), Some(expected)) => {
                    assert_eq!(actual.balance, expected.balance);
                    assert_eq!(actual.nonce, expected.nonce);
                    assert_eq!(actual.code_hash, expected.code_hash);
                    assert_eq!(actual.storage_root, expected.storage_root);
                    assert_eq!(reconstructor.storage_root(address), expected.storage_root);
                }
                (None, None) => {
                    assert_eq!(reconstructor.storage_root(address), EMPTY_ROOT);
                }
                (actual, expected) => panic!(
                    "account mismatch for {address:?}: actual={actual:?} expected={expected:?}"
                ),
            }
        }

        for (address, slot_key) in expected_slots {
            let expected_value = expected_state
                .storage
                .get(address)
                .and_then(|storage| storage.get(slot_key))
                .copied()
                .unwrap_or(U256::ZERO);
            assert_eq!(
                reconstructor.storage_slot(*address, *slot_key),
                expected_value,
                "slot mismatch for address {address:?} slot {slot_key:?}"
            );
        }

        for (code_hash, expected_bytecode) in expected_bytecodes {
            assert_eq!(
                diff.deployed_bytecodes
                    .get(code_hash)
                    .map(|bytes| bytes.as_ref()),
                Some(*expected_bytecode)
            );
        }
    }

    fn roundtrip_batch_diff(blocks: &[BlockStateChanges]) -> BatchStateDiff {
        let diff = batch_diff(blocks);
        let encoded = encode_to_vec(&diff).unwrap();
        decode_buf_exact(&encoded).unwrap()
    }

    #[test]
    fn test_reconstruct_storage_only_change_matches_canonical_oracle() {
        let address = addr(0x11);
        let slot_one = slot(1);
        let slot_two = slot(2);
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(100, 2, hash(0x21)))
            .set_storage_slot(address, slot_one, value(10));
        let expected_state = pre_state
            .clone()
            .set_storage_slot(address, slot_one, value(11))
            .set_storage_slot(address, slot_two, value(22));

        let mut block = block_diff();
        storage_change(&mut block, address, slot_one, value(10), value(11));
        storage_change(&mut block, address, slot_two, U256::ZERO, value(22));

        let diff = roundtrip_batch_diff(&[block]);
        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, slot_one), (address, slot_two)],
            &[],
            &diff,
        );
    }

    #[test]
    fn test_reconstruct_zero_slot_reset_matches_canonical_oracle() {
        let address = addr(0x12);
        let slot_one = slot(1);
        let slot_two = slot(2);
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(250, 3, hash(0x22)))
            .set_storage_slot(address, slot_one, value(5))
            .set_storage_slot(address, slot_two, value(8));
        let expected_state = pre_state.clone().remove_storage_slot(address, slot_one);

        let mut block = block_diff();
        storage_change(&mut block, address, slot_one, value(5), U256::ZERO);

        let diff = roundtrip_batch_diff(&[block]);
        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, slot_one), (address, slot_two)],
            &[],
            &diff,
        );
    }

    #[test]
    fn test_reconstruct_created_then_deleted_matches_canonical_oracle() {
        let address = addr(0x13);
        let slot_one = slot(1);
        let pre_state = CanonicalState::new();
        let expected_state = CanonicalState::new();

        let mut block_one = block_diff();
        account_change(
            &mut block_one,
            address,
            None,
            Some(snapshot(75, 1, hash(0x23))),
        );
        storage_change(&mut block_one, address, slot_one, U256::ZERO, value(9));

        let mut block_two = block_diff();
        account_change(
            &mut block_two,
            address,
            Some(snapshot(75, 1, hash(0x23))),
            None,
        );
        storage_change(&mut block_two, address, slot_one, value(9), U256::ZERO);

        let diff = roundtrip_batch_diff(&[block_one, block_two]);
        assert!(diff.is_empty());

        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, slot_one)],
            &[],
            &diff,
        );
    }

    #[test]
    fn test_reconstruct_mid_batch_revert_matches_canonical_oracle() {
        let address = addr(0x14);
        let slot_one = slot(1);
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(100, 4, hash(0x24)))
            .set_storage_slot(address, slot_one, value(5));
        let expected_state = pre_state.clone();

        let mut block_one = block_diff();
        account_change(
            &mut block_one,
            address,
            Some(snapshot(100, 4, hash(0x24))),
            Some(snapshot(150, 5, hash(0x24))),
        );
        storage_change(&mut block_one, address, slot_one, value(5), value(6));

        let mut block_two = block_diff();
        account_change(
            &mut block_two,
            address,
            Some(snapshot(150, 5, hash(0x24))),
            Some(snapshot(100, 4, hash(0x24))),
        );
        storage_change(&mut block_two, address, slot_one, value(6), value(5));

        let diff = roundtrip_batch_diff(&[block_one, block_two]);
        assert!(diff.is_empty());

        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, slot_one)],
            &[],
            &diff,
        );
    }

    #[test]
    fn test_reconstruct_code_churn_matches_canonical_oracle() {
        let address = addr(0x15);
        let slot_one = slot(1);
        let old_hash = hash(0x25);
        let new_hash = hash(0x26);
        let new_bytecode = [0x60, 0x80, 0x60, 0x40, 0x52];
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(500, 8, old_hash))
            .set_storage_slot(address, slot_one, value(1));
        let expected_state = CanonicalState::new()
            .with_account(address, state_account(500, 8, new_hash))
            .set_storage_slot(address, slot_one, value(3));

        let mut block = block_diff();
        account_change(
            &mut block,
            address,
            Some(snapshot(500, 8, old_hash)),
            Some(snapshot(500, 8, new_hash)),
        );
        storage_change(&mut block, address, slot_one, value(1), value(3));
        deployed_bytecode(&mut block, new_hash, bytecode(&new_bytecode));

        let diff = roundtrip_batch_diff(&[block]);
        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, slot_one)],
            &[(new_hash, &new_bytecode)],
            &diff,
        );
    }

    #[test]
    fn test_reconstruct_selfdestruct_recreate_matches_canonical_oracle() {
        let address = addr(0x16);
        let old_hash = hash(0x27);
        let new_hash = hash(0x28);
        let old_slot = slot(1);
        let new_slot = slot(2);
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(900, 7, old_hash))
            .set_storage_slot(address, old_slot, value(33));
        let expected_state = CanonicalState::new()
            .with_account(address, state_account(55, 1, new_hash))
            .set_storage_slot(address, new_slot, value(44));

        let mut block_one = block_diff();
        account_change(
            &mut block_one,
            address,
            Some(snapshot(900, 7, old_hash)),
            None,
        );
        storage_change(&mut block_one, address, old_slot, value(33), U256::ZERO);

        let mut block_two = block_diff();
        account_change(
            &mut block_two,
            address,
            None,
            Some(snapshot(55, 1, new_hash)),
        );
        storage_change(&mut block_two, address, new_slot, U256::ZERO, value(44));

        let diff = roundtrip_batch_diff(&[block_one, block_two]);
        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        assert_reconstruction_matches(
            &reconstructor,
            &expected_state,
            &[(address, old_slot), (address, new_slot)],
            &[],
            &diff,
        );
    }

    proptest! {
        #[test]
        fn proptest_batch_builder_elides_reverted_changes(
            initial_balance in 1u64..10_000,
            initial_nonce in 1u64..100,
            changed_balance in 1u64..10_000,
            changed_nonce in 1u64..100,
            initial_slot in 0u64..500,
            changed_slot in 0u64..500,
        ) {
            let address = addr(0x31);
            let slot_key = U256::from(1);
            let code_hash = hash(0x41);

            prop_assume!(
                initial_balance != changed_balance
                    || initial_nonce != changed_nonce
                    || initial_slot != changed_slot
            );

            let mut block_one = block_diff();
            account_change(
                &mut block_one,
                address,
                Some(snapshot(initial_balance, initial_nonce, code_hash)),
                Some(snapshot(changed_balance, changed_nonce, code_hash)),
            );
            storage_change(
                &mut block_one,
                address,
                slot_key,
                U256::from(initial_slot),
                U256::from(changed_slot),
            );

            let mut block_two = block_diff();
            account_change(
                &mut block_two,
                address,
                Some(snapshot(changed_balance, changed_nonce, code_hash)),
                Some(snapshot(initial_balance, initial_nonce, code_hash)),
            );
            storage_change(
                &mut block_two,
                address,
                slot_key,
                U256::from(changed_slot),
                U256::from(initial_slot),
            );

            let diff = batch_diff(&[block_one, block_two]);
            prop_assert!(diff.is_empty());
        }

        #[test]
        fn proptest_batch_state_diff_encoding_is_deterministic(
            balance in 1u64..10_000,
            nonce in 1u64..100,
            slot_before in 0u64..500,
            slot_after in 0u64..500,
        ) {
            let address = addr(0x32);
            let code_hash = hash(0x42);
            let slot_key = U256::from(1);

            let mut block = block_diff();
            account_change(
                &mut block,
                address,
                Some(snapshot(balance, nonce, code_hash)),
                Some(snapshot(balance.saturating_add(1), nonce.saturating_add(1), code_hash)),
            );
            storage_change(
                &mut block,
                address,
                slot_key,
                U256::from(slot_before),
                U256::from(slot_after),
            );

            let first = encode_to_vec(&batch_diff(&[block.clone()])).unwrap();
            let second = encode_to_vec(&batch_diff(&[block])).unwrap();
            prop_assert_eq!(first, second);
        }

        #[test]
        fn proptest_reconstruction_matches_canonical_oracle(
            pre_balance in 1u64..10_000,
            post_balance in 1u64..10_000,
            pre_nonce in 1u64..100,
            post_nonce in 1u64..100,
            pre_slot in 0u64..500,
            post_slot in 0u64..500,
        ) {
            let address = addr(0x33);
            let code_hash = hash(0x43);
            let slot_key = U256::from(1);
            let pre_state = CanonicalState::new()
                .with_account(address, state_account(pre_balance, pre_nonce, code_hash))
                .set_storage_slot(address, slot_key, U256::from(pre_slot));

            let expected_state = if post_slot == 0 {
                CanonicalState::new()
                    .with_account(address, state_account(post_balance, post_nonce, code_hash))
                    .remove_storage_slot(address, slot_key)
            } else {
                CanonicalState::new()
                    .with_account(address, state_account(post_balance, post_nonce, code_hash))
                    .set_storage_slot(address, slot_key, U256::from(post_slot))
            };

            let mut block = block_diff();
            account_change(
                &mut block,
                address,
                Some(snapshot(pre_balance, pre_nonce, code_hash)),
                Some(snapshot(post_balance, post_nonce, code_hash)),
            );
            storage_change(
                &mut block,
                address,
                slot_key,
                U256::from(pre_slot),
                U256::from(post_slot),
            );

            let diff = roundtrip_batch_diff(&[block]);
            let mut reconstructor =
                StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
            reconstructor.apply_diff(&diff).unwrap();

            assert_reconstruction_matches(
                &reconstructor,
                &expected_state,
                &[(address, slot_key)],
                &[],
                &diff,
            );
        }
    }

    /// Cross-verifies that [`apply_batch_state_diff_to_ethereum_state`]
    /// (acct-guest path) produces the same post-state root as
    /// [`StateReconstructor::apply_diff`] (existing oracle path) when both
    /// start from the same empty state and consume the same diff.
    ///
    /// Empty start is sufficient here: the diff's `Created` variants
    /// produce all the post-state, no pre-state is required. Tests that
    /// exercise populated pre-state for `EthereumState` live alongside the
    /// chunk pipeline where the witness-extraction wiring exists.
    #[test]
    fn apply_to_ethereum_state_matches_state_reconstructor_oracle() {
        let address_a = addr(0xA1);
        let address_b = addr(0xB2);
        let slot_one = slot(1);
        let slot_two = slot(2);

        let pre_state = CanonicalState::new();
        let expected_state = CanonicalState::new()
            .with_account(address_a, state_account(500, 1, hash(0x33)))
            .set_storage_slot(address_a, slot_one, value(100))
            .with_account(address_b, state_account(750, 2, hash(0x44)))
            .set_storage_slot(address_b, slot_two, value(200));

        let mut block = block_diff();
        account_change(
            &mut block,
            address_a,
            None,
            Some(snapshot(500, 1, hash(0x33))),
        );
        storage_change(&mut block, address_a, slot_one, U256::ZERO, value(100));
        account_change(
            &mut block,
            address_b,
            None,
            Some(snapshot(750, 2, hash(0x44))),
        );
        storage_change(&mut block, address_b, slot_two, U256::ZERO, value(200));

        let diff = roundtrip_batch_diff(&[block]);

        // Oracle path.
        let mut reconstructor =
            StateReconstructor::from_state_parts(&pre_state.accounts, &pre_state.storage).unwrap();
        reconstructor.apply_diff(&diff).unwrap();

        // New path: apply to a fresh empty EthereumState — `EthereumState`
        // has no `Default` impl, so build via its public fields.
        let mut state = EthereumState {
            state_trie: Default::default(),
            storage_tries: Default::default(),
        };
        apply_batch_state_diff_to_ethereum_state(&mut state, &diff).unwrap();

        assert_eq!(
            reconstructor.state_root(),
            state.state_root(),
            "ethereum-state apply must agree with reconstructor oracle"
        );
        assert_eq!(
            state.state_root(),
            canonical_state_root(&expected_state).unwrap(),
            "ethereum-state apply must match canonical post-state root"
        );
    }
}
