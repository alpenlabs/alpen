//! State reconstruction from batch diffs.

use std::collections::{BTreeMap, HashMap};

#[cfg(feature = "chainspec")]
use alpen_chainspec::chain_value_parser;
use revm_primitives::{alloy_primitives::Address, B256, U256};
use rsp_mpt::EthereumState;
use serde::{Deserialize, Serialize};
use strata_da_framework::ContextlessDaWrite;
#[cfg(feature = "chainspec")]
use strata_mpt::KECCAK_EMPTY;
use strata_mpt::{keccak, MptNode, StateAccount, EMPTY_ROOT};

use crate::{
    batch::{AccountChange, BatchStateDiff, StorageDiff},
    block::AccountSnapshot,
};

/// Error that may occur during state reconstruction.
#[derive(Debug, thiserror::Error)]
pub enum ReconstructError {
    #[error("MPT: {0}")]
    Mpt(#[from] strata_mpt::Error),
    #[error("sparse MPT: {0}")]
    SparseMpt(#[from] rsp_mpt::Error),
    #[error("DA apply: {0}")]
    Da(#[from] strata_da_framework::DaError),
}

/// Canonical account and storage state used to initialize a [`StateReconstructor`].
///
/// The prestate stores account records separately from per-account storage. During
/// initialization, the reconstructor recomputes each account's storage root from the
/// supplied storage map before inserting the account into the global state trie.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateReconstructorPreState {
    accounts: BTreeMap<Address, StateAccount>,
    storage: BTreeMap<Address, BTreeMap<U256, U256>>,
}

impl StateReconstructorPreState {
    /// Creates a prestate from canonical account and storage state.
    pub fn new(
        accounts: BTreeMap<Address, StateAccount>,
        storage: BTreeMap<Address, BTreeMap<U256, U256>>,
    ) -> Self {
        Self { accounts, storage }
    }

    /// Returns the canonical account records keyed by address.
    pub fn accounts(&self) -> &BTreeMap<Address, StateAccount> {
        &self.accounts
    }

    /// Returns the canonical storage slots keyed by address and slot.
    pub fn storage(&self) -> &BTreeMap<Address, BTreeMap<U256, U256>> {
        &self.storage
    }

    /// Consumes the prestate and returns its canonical account and storage parts.
    pub fn into_parts(
        self,
    ) -> (
        BTreeMap<Address, StateAccount>,
        BTreeMap<Address, BTreeMap<U256, U256>>,
    ) {
        (self.accounts, self.storage)
    }
}

/// Reconstructs EVM state by applying [`BatchStateDiff`]s sequentially.
///
/// Used primarily for testing to verify that state roots reconstructed
/// from diffs match the actual state roots from EE blocks.
///
/// Maintains canonical account and storage maps alongside the MPTs to support
/// cheap export through [`StateReconstructor::to_prestate`]. This roughly doubles
/// state memory, and every mutation must keep the maps and MPTs in lock-step.
#[derive(Clone, Default, Debug)]
pub struct StateReconstructor {
    state_trie: MptNode,
    storage_trie: HashMap<Address, MptNode>,
    accounts: BTreeMap<Address, StateAccount>,
    storage: BTreeMap<Address, BTreeMap<U256, U256>>,
}

impl StateReconstructor {
    /// Creates a new empty reconstructor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a reconstructor initialized with genesis state from a chain spec.
    #[cfg(feature = "chainspec")]
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
                    let (storage_root, storage_trie_is_empty, acc_storage_is_empty) = {
                        let acc_storage_trie =
                            reconstructor.storage_trie.entry(*address).or_default();
                        let acc_storage = reconstructor.storage.entry(*address).or_default();
                        for (slot_key, slot_value) in slots.iter() {
                            if slot_value != &B256::ZERO {
                                // Chain specs key storage as raw 32-byte words; prestates use
                                // U256 to match state-diff storage keys.
                                let slot_key = U256::from_be_bytes(slot_key.0);
                                let slot_value = U256::from_be_bytes(slot_value.0);
                                acc_storage_trie.insert_rlp(
                                    &keccak(slot_key.to_be_bytes::<32>()),
                                    slot_value,
                                )?;
                                acc_storage.insert(slot_key, slot_value);
                            }
                        }
                        (
                            acc_storage_trie.hash(),
                            acc_storage_trie.is_empty(),
                            acc_storage.is_empty(),
                        )
                    };

                    state_account.storage_root = storage_root;
                    if acc_storage_is_empty {
                        reconstructor.storage.remove(address);
                    }
                    if storage_trie_is_empty {
                        reconstructor.storage_trie.remove(address);
                    }
                }
            }

            reconstructor
                .state_trie
                .insert_rlp(&keccak(address), state_account.clone())?;
            reconstructor.accounts.insert(*address, state_account);
        }

        Ok(reconstructor)
    }

    /// Creates a reconstructor initialized with explicit canonical state.
    ///
    /// Empty accounts are skipped during initialization. Each account's storage root
    /// is recomputed from the prestate's storage map before the account is inserted
    /// into the state trie.
    pub fn from_prestate(prestate: &StateReconstructorPreState) -> Result<Self, ReconstructError> {
        let mut reconstructor = Self::new();

        for (address, account) in prestate.accounts() {
            let mut state_account = account.clone();
            if state_account.is_account_empty() {
                continue;
            }

            let mut storage_trie = MptNode::default();
            let mut account_storage = BTreeMap::new();

            if let Some(prestate_storage) = prestate.storage().get(address) {
                for (slot_key, slot_value) in prestate_storage {
                    if slot_value.is_zero() {
                        continue;
                    }

                    storage_trie.insert_rlp(&keccak(slot_key.to_be_bytes::<32>()), *slot_value)?;
                    account_storage.insert(*slot_key, *slot_value);
                }
            }

            state_account.storage_root = storage_trie.hash();
            if !storage_trie.is_empty() {
                reconstructor.storage_trie.insert(*address, storage_trie);
            }
            if !account_storage.is_empty() {
                reconstructor.storage.insert(*address, account_storage);
            }

            reconstructor
                .state_trie
                .insert_rlp(&keccak(address), state_account.clone())?;
            reconstructor.accounts.insert(*address, state_account);
        }

        Ok(reconstructor)
    }

    /// Exports the current canonical state as a reconstructor prestate.
    ///
    /// Empty accounts and zero-valued storage slots are not represented in the
    /// returned prestate.
    pub fn to_prestate(&self) -> StateReconstructorPreState {
        StateReconstructorPreState::new(self.accounts.clone(), self.storage.clone())
    }

    /// Applies a [`BatchStateDiff`] to the current state.
    pub fn apply_diff(&mut self, diff: &BatchStateDiff) -> Result<(), ReconstructError> {
        for (address, change) in &diff.accounts {
            let acc_info_trie_path = keccak(address);

            match change {
                AccountChange::Created(account_diff) | AccountChange::Updated(account_diff) => {
                    // Get current account state (if exists)
                    let current = self.accounts.get(address).cloned();

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
                    state_account.storage_root = match diff.storage.get(address) {
                        Some(storage_diff) => self.apply_storage_diff(*address, storage_diff)?,
                        None => self.compute_storage_root(*address),
                    };

                    self.state_trie
                        .insert_rlp(&acc_info_trie_path, state_account.clone())?;
                    self.accounts.insert(*address, state_account);
                }
                AccountChange::Deleted => {
                    self.state_trie.delete(&acc_info_trie_path)?;
                    self.storage_trie.remove(address);
                    self.accounts.remove(address);
                    self.storage.remove(address);
                }
            }
        }

        // (e.g., storage-only changes)
        for (address, storage_diff) in &diff.storage {
            if diff.accounts.contains_key(address) {
                continue; // Already handled above
            }

            let acc_info_trie_path = keccak(address);
            let current = self.accounts.get(address).cloned();

            if let Some(mut state_account) = current {
                state_account.storage_root = self.apply_storage_diff(*address, storage_diff)?;
                self.state_trie
                    .insert_rlp(&acc_info_trie_path, state_account.clone())?;
                self.accounts.insert(*address, state_account);
            }
        }

        Ok(())
    }

    fn apply_storage_diff(
        &mut self,
        address: Address,
        storage_diff: &StorageDiff,
    ) -> Result<B256, ReconstructError> {
        let (storage_root, storage_trie_is_empty, account_storage_is_empty) = {
            let acc_storage_trie = self.storage_trie.entry(address).or_default();
            let account_storage = self.storage.entry(address).or_default();

            for (slot_key, slot_value) in storage_diff.iter() {
                let slot_trie_path = keccak(slot_key.to_be_bytes::<32>());
                match slot_value {
                    Some(v) if !v.is_zero() => {
                        acc_storage_trie.insert_rlp(&slot_trie_path, *v)?;
                        account_storage.insert(*slot_key, *v);
                    }
                    _ => {
                        acc_storage_trie.delete(&slot_trie_path)?;
                        account_storage.remove(slot_key);
                    }
                }
            }

            (
                acc_storage_trie.hash(),
                acc_storage_trie.is_empty(),
                account_storage.is_empty(),
            )
        };

        if storage_trie_is_empty {
            self.storage_trie.remove(&address);
        }
        if account_storage_is_empty {
            self.storage.remove(&address);
        }

        Ok(storage_root)
    }

    /// Computes the current state root.
    pub fn compute_state_root(&self) -> B256 {
        self.state_trie.hash()
    }

    /// Computes the current storage root for an account.
    ///
    /// Returns [`EMPTY_ROOT`] when no storage trie is known for `address`. This is
    /// an accessor over reconstructed state, not an absence proof for a claimed
    /// state root.
    pub fn compute_storage_root(&self, address: Address) -> B256 {
        self.storage_trie
            .get(&address)
            .map(|t| t.hash())
            .unwrap_or(EMPTY_ROOT)
    }

    /// Returns the value at a storage slot.
    ///
    /// Returns zero when the account or slot is absent. Callers that need to
    /// distinguish absent storage from an explicit zero value must use a richer
    /// state witness than this reconstructed map.
    pub fn get_storage_slot(&self, address: Address, slot_key: U256) -> U256 {
        self.storage
            .get(&address)
            .and_then(|account_storage| account_storage.get(&slot_key))
            .copied()
            .unwrap_or_default()
    }

    /// Returns the account state when it is present in reconstructed state.
    ///
    /// `None` means the reconstructor has no account for `address`; it is not an
    /// absence proof against an externally supplied state root.
    pub fn get_account(&self, address: Address) -> Option<StateAccount> {
        self.accounts.get(&address).cloned()
    }
}

/// Applies a [`BatchStateDiff`] to a populated [`EthereumState`] sparse-MPT witness.
///
/// This mirrors [`StateReconstructor::apply_diff`] but operates on the sparse MPT
/// shape consumed by the EVM chunk witness pipeline, allowing the acct proof to
/// apply a DA-published state diff to the same pre-state witness used for execution.
pub fn apply_batch_state_diff_to_ethereum_state(
    state: &mut EthereumState,
    diff: &BatchStateDiff,
) -> Result<(), ReconstructError> {
    for (address, change) in &diff.accounts {
        let hashed_addr: B256 = keccak(address).into();

        match change {
            AccountChange::Created(account_diff) | AccountChange::Updated(account_diff) => {
                let current: Option<StateAccount> =
                    state.state_trie.get_rlp(hashed_addr.as_slice())?;

                let mut snapshot = current
                    .as_ref()
                    .map(AccountSnapshot::from)
                    .unwrap_or_default();

                account_diff.apply(&mut snapshot)?;

                let mut state_account = StateAccount {
                    nonce: snapshot.nonce,
                    balance: snapshot.balance,
                    storage_root: current
                        .as_ref()
                        .map(|account| account.storage_root)
                        .unwrap_or(EMPTY_ROOT),
                    code_hash: snapshot.code_hash,
                };

                if state_account.is_account_empty() {
                    continue;
                }

                if let Some(storage_diff) = diff.storage.get(address) {
                    let acc_storage_trie = state.storage_tries.entry(hashed_addr).or_default();
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
                }

                state
                    .state_trie
                    .insert_rlp(hashed_addr.as_slice(), state_account)?;
            }
            AccountChange::Deleted => {
                state.state_trie.delete(hashed_addr.as_slice())?;
                state.storage_tries.remove(&hashed_addr);
            }
        }
    }

    for (address, storage_diff) in &diff.storage {
        if diff.accounts.contains_key(address) {
            continue;
        }

        let hashed_addr: B256 = keccak(address).into();
        let current: Option<StateAccount> = state.state_trie.get_rlp(hashed_addr.as_slice())?;

        if let Some(mut state_account) = current {
            let acc_storage_trie = state.storage_tries.entry(hashed_addr).or_default();
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
            state
                .state_trie
                .insert_rlp(hashed_addr.as_slice(), state_account)?;
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
    fn reconstructor_from_state(state: &CanonicalState) -> StateReconstructor {
        let prestate =
            StateReconstructorPreState::new(state.accounts.clone(), state.storage.clone());
        StateReconstructor::from_prestate(&prestate).expect("prestate must reconstruct")
    }

    fn assert_reconstruction_matches(
        reconstructor: &StateReconstructor,
        expected_state: &CanonicalState,
        expected_slots: &[(Address, U256)],
        expected_bytecodes: &[(B256, &[u8])],
        diff: &BatchStateDiff,
    ) {
        let expected_accounts = canonical_accounts(expected_state).unwrap();
        assert_eq!(
            reconstructor.compute_state_root(),
            canonical_state_root(expected_state).unwrap()
        );

        let addresses = expected_state
            .accounts
            .keys()
            .chain(expected_state.storage.keys())
            .copied()
            .collect::<BTreeSet<_>>();

        for address in addresses {
            let actual_account = reconstructor.get_account(address);
            let expected_account = expected_accounts.get(&address);

            match (actual_account, expected_account) {
                (Some(actual), Some(expected)) => {
                    assert_eq!(actual.balance, expected.balance);
                    assert_eq!(actual.nonce, expected.nonce);
                    assert_eq!(actual.code_hash, expected.code_hash);
                    assert_eq!(actual.storage_root, expected.storage_root);
                    assert_eq!(
                        reconstructor.compute_storage_root(address),
                        expected.storage_root
                    );
                }
                (None, None) => {
                    assert_eq!(reconstructor.compute_storage_root(address), EMPTY_ROOT);
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
                reconstructor.get_storage_slot(*address, *slot_key),
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
    fn test_reconstructor_prestate_export_roundtrips_canonical_state() {
        let address = addr(0x10);
        let slot_one = slot(1);
        let canonical_state = CanonicalState::new()
            .with_account(address, state_account(100, 1, hash(0x20)))
            .set_storage_slot(address, slot_one, value(7));

        let reconstructor = reconstructor_from_state(&canonical_state);
        let prestate = reconstructor.to_prestate();
        let from_prestate = StateReconstructor::from_prestate(&prestate).unwrap();

        assert_eq!(
            from_prestate.compute_state_root(),
            reconstructor.compute_state_root()
        );
        assert_eq!(
            prestate.accounts(),
            &canonical_accounts(&canonical_state).unwrap()
        );
        assert_eq!(prestate.storage(), &canonical_state.storage);
    }

    #[test]
    fn test_reconstructor_prestate_json_roundtrip_preserves_state() {
        let address = addr(0x18);
        let slot_one = slot(1);
        let canonical_state = CanonicalState::new()
            .with_account(address, state_account(250, 3, hash(0x22)))
            .set_storage_slot(address, slot_one, value(12));
        let prestate = StateReconstructorPreState::new(
            canonical_accounts(&canonical_state).unwrap(),
            canonical_state.storage.clone(),
        );

        let encoded = serde_json::to_string(&prestate).expect("prestate serializes");
        let decoded: StateReconstructorPreState =
            serde_json::from_str(&encoded).expect("prestate deserializes");

        assert_eq!(decoded, prestate);
        assert_eq!(
            StateReconstructor::from_prestate(&decoded)
                .expect("decoded prestate reconstructs")
                .compute_state_root(),
            canonical_state_root(&canonical_state).unwrap()
        );
    }

    #[test]
    fn test_reconstructor_prestate_export_tracks_applied_diff() {
        let address = addr(0x17);
        let slot_one = slot(1);
        let slot_two = slot(2);
        let pre_state = CanonicalState::new()
            .with_account(address, state_account(100, 1, hash(0x21)))
            .set_storage_slot(address, slot_one, value(5));
        let expected_state = CanonicalState::new()
            .with_account(address, state_account(150, 2, hash(0x21)))
            .set_storage_slot(address, slot_two, value(9));

        let mut block = block_diff();
        account_change(
            &mut block,
            address,
            Some(snapshot(100, 1, hash(0x21))),
            Some(snapshot(150, 2, hash(0x21))),
        );
        storage_change(&mut block, address, slot_one, value(5), U256::ZERO);
        storage_change(&mut block, address, slot_two, U256::ZERO, value(9));

        let diff = roundtrip_batch_diff(&[block]);
        let mut reconstructor = reconstructor_from_state(&pre_state);
        reconstructor.apply_diff(&diff).unwrap();
        let prestate = reconstructor.to_prestate();
        let from_prestate = StateReconstructor::from_prestate(&prestate).unwrap();

        assert_eq!(
            from_prestate.compute_state_root(),
            reconstructor.compute_state_root()
        );
        assert_eq!(
            prestate.accounts(),
            &canonical_accounts(&expected_state).unwrap()
        );
        assert_eq!(prestate.storage(), &expected_state.storage);
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
        let mut reconstructor = reconstructor_from_state(&pre_state);
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
        let mut reconstructor = reconstructor_from_state(&pre_state);
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

        let mut reconstructor = reconstructor_from_state(&pre_state);
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

        let mut reconstructor = reconstructor_from_state(&pre_state);
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
        let mut reconstructor = reconstructor_from_state(&pre_state);
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
        let mut reconstructor = reconstructor_from_state(&pre_state);
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
            let mut reconstructor = reconstructor_from_state(&pre_state);
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
    /// produces the same post-state root as [`StateReconstructor::apply_diff`]
    /// when both start from the same empty state and consume the same diff.
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

        let mut reconstructor = reconstructor_from_state(&pre_state);
        reconstructor.apply_diff(&diff).unwrap();

        let mut state = EthereumState {
            state_trie: Default::default(),
            storage_tries: Default::default(),
        };
        apply_batch_state_diff_to_ethereum_state(&mut state, &diff).unwrap();

        assert_eq!(
            reconstructor.compute_state_root(),
            state.state_root(),
            "ethereum-state apply must agree with reconstructor oracle"
        );
        assert_eq!(
            state.state_root(),
            canonical_state_root(&expected_state).unwrap(),
            "ethereum-state apply must match canonical post-state root"
        );
    }

    #[test]
    fn apply_to_ethereum_state_returns_mpt_error_for_unresolved_state_trie() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let address = addr(0xC1);
        let unresolved = hash(0xFE);

        let mut block = block_diff();
        account_change(
            &mut block,
            address,
            None,
            Some(snapshot(500, 1, hash(0x55))),
        );
        let diff = roundtrip_batch_diff(&[block]);

        let mut state = EthereumState::from_proofs(unresolved, &Default::default()).unwrap();

        let result = catch_unwind(AssertUnwindSafe(|| {
            apply_batch_state_diff_to_ethereum_state(&mut state, &diff)
        }));

        assert!(result.is_ok(), "unresolved sparse trie must not panic");
        let err = result
            .unwrap()
            .expect_err("unresolved sparse trie must return an MPT error");
        assert!(matches!(
            err,
            ReconstructError::SparseMpt(rsp_mpt::Error::NodeNotResolved(digest))
                if digest == unresolved
        ));
    }
}
