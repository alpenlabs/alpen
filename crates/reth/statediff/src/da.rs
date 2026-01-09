//! DA-framework based types for efficient EE state diff encoding.
//!
//! This module provides space-efficient encoding of EE state changes using the
//! `strata-da-framework` primitives:
//! - `DaRegister` for values that are wholly replaced
//! - `DaCounter` for values that increment/decrement by small amounts
//! - Compound types for combining multiple primitives
//!
//! The goal is to minimize DA posting costs by encoding only what changed,
//! using delta encoding where applicable.

use std::collections::BTreeMap;

use alloy_primitives::U256;
use revm_primitives::{Address, B256, KECCAK_EMPTY};
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::{BuilderError, ContextlessDaWrite, DaError, DaRegister, DaWrite};

use revm::database::{BundleAccount, BundleState};

use crate::BatchStateDiff;

// ============================================================================
// Codec wrappers for Alloy types
// ============================================================================

/// Wrapper for U256 that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecU256(pub U256);

impl Codec for CodecU256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&self.0.to_le_bytes::<32>())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf)?;
        Ok(Self(U256::from_le_bytes(buf)))
    }
}

impl From<U256> for CodecU256 {
    fn from(v: U256) -> Self {
        Self(v)
    }
}

impl From<CodecU256> for U256 {
    fn from(v: CodecU256) -> Self {
        v.0
    }
}

/// Wrapper for B256 that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecB256(pub B256);

impl Codec for CodecB256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf)?;
        Ok(Self(B256::from(buf)))
    }
}

impl From<B256> for CodecB256 {
    fn from(v: B256) -> Self {
        Self(v)
    }
}

impl From<CodecB256> for B256 {
    fn from(v: CodecB256) -> Self {
        v.0
    }
}

/// Wrapper for Address that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CodecAddress(pub Address);

impl Codec for CodecAddress {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 20];
        dec.read_buf(&mut buf)?;
        Ok(Self(Address::from(buf)))
    }
}

impl From<Address> for CodecAddress {
    fn from(v: Address) -> Self {
        Self(v)
    }
}

impl From<CodecAddress> for Address {
    fn from(v: CodecAddress) -> Self {
        v.0
    }
}

// ============================================================================
// Account state for DA application
// ============================================================================

/// Represents the EE account state that DA diffs are applied to.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaAccountState {
    pub balance: CodecU256,
    pub nonce: u64,
    pub code_hash: CodecB256,
}

impl DaAccountState {
    pub fn new(balance: U256, nonce: u64, code_hash: B256) -> Self {
        Self {
            balance: CodecU256(balance),
            nonce,
            code_hash: CodecB256(code_hash),
        }
    }
}

// ============================================================================
// Account Diff using DA primitives
// ============================================================================

/// Diff for a single account using DA framework primitives.
///
/// - `balance`: Register (can change arbitrarily)
/// - `nonce`: Stored as `Option<u8>` (nonces typically increment by small amounts)
/// - `code_hash`: Register (only changes on contract creation)
#[derive(Clone, Debug, Default)]
pub struct DaAccountDiff {
    /// Balance change (full replacement if changed).
    pub balance: DaRegister<CodecU256>,
    /// Nonce increment (None = unchanged, Some(n) = increment by n).
    pub nonce_incr: Option<u8>,
    /// Code hash change (only on contract creation).
    pub code_hash: DaRegister<CodecB256>,
}

/// Serde-friendly representation of DaAccountDiff for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaAccountDiffSerde {
    /// New balance value (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<U256>,
    /// Nonce increment (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce_incr: Option<u8>,
    /// New code hash (None = unchanged).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_hash: Option<B256>,
}

impl From<&DaAccountDiff> for DaAccountDiffSerde {
    fn from(diff: &DaAccountDiff) -> Self {
        Self {
            balance: diff.balance.new_value().map(|v| v.0),
            nonce_incr: diff.nonce_incr,
            code_hash: diff.code_hash.new_value().map(|v| v.0),
        }
    }
}

impl From<DaAccountDiffSerde> for DaAccountDiff {
    fn from(serde: DaAccountDiffSerde) -> Self {
        Self {
            balance: serde
                .balance
                .map(|v| DaRegister::new_set(CodecU256(v)))
                .unwrap_or_else(DaRegister::new_unset),
            nonce_incr: serde.nonce_incr,
            code_hash: serde
                .code_hash
                .map(|v| DaRegister::new_set(CodecB256(v)))
                .unwrap_or_else(DaRegister::new_unset),
        }
    }
}

impl DaAccountDiff {
    /// Creates a new account diff with all fields unchanged.
    pub fn new_unchanged() -> Self {
        Self::default()
    }

    /// Creates a diff representing a new account creation.
    pub fn new_created(balance: U256, nonce: u64, code_hash: B256) -> Self {
        Self {
            balance: DaRegister::new_set(CodecU256(balance)),
            nonce_incr: if nonce > 0 { Some(nonce as u8) } else { None },
            code_hash: DaRegister::new_set(CodecB256(code_hash)),
        }
    }

    /// Creates a diff by comparing original and new account states.
    pub fn from_change(
        original: &DaAccountState,
        new: &DaAccountState,
    ) -> Result<Self, BuilderError> {
        let balance = DaRegister::compare(&original.balance, &new.balance);

        // For nonce, compute the increment
        let nonce_diff = new.nonce.saturating_sub(original.nonce);
        let nonce_incr = if nonce_diff == 0 {
            None
        } else if nonce_diff <= u8::MAX as u64 {
            Some(nonce_diff as u8)
        } else {
            return Err(BuilderError::OutOfBoundsValue);
        };

        let code_hash = DaRegister::compare(&original.code_hash, &new.code_hash);

        Ok(Self {
            balance,
            nonce_incr,
            code_hash,
        })
    }

    /// Returns true if no changes are recorded.
    pub fn is_unchanged(&self) -> bool {
        DaWrite::is_default(&self.balance)
            && self.nonce_incr.is_none()
            && DaWrite::is_default(&self.code_hash)
    }
}

impl DaWrite for DaAccountDiff {
    type Target = DaAccountState;
    type Context = ();

    fn is_default(&self) -> bool {
        self.is_unchanged()
    }

    fn apply(&self, target: &mut Self::Target, _context: &Self::Context) -> Result<(), DaError> {
        ContextlessDaWrite::apply(&self.balance, &mut target.balance)?;
        if let Some(incr) = self.nonce_incr {
            target.nonce = target.nonce.wrapping_add(incr as u64);
        }
        ContextlessDaWrite::apply(&self.code_hash, &mut target.code_hash)?;
        Ok(())
    }
}

impl Codec for DaAccountDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Use a bitmap to track which fields are set (3 bits needed)
        let mut bitmap: u8 = 0;
        if !DaWrite::is_default(&self.balance) {
            bitmap |= 1;
        }
        if self.nonce_incr.is_some() {
            bitmap |= 2;
        }
        if !DaWrite::is_default(&self.code_hash) {
            bitmap |= 4;
        }

        bitmap.encode(enc)?;

        // Only encode non-default fields
        if !DaWrite::is_default(&self.balance) {
            self.balance.new_value().unwrap().encode(enc)?;
        }
        if let Some(incr) = self.nonce_incr {
            incr.encode(enc)?;
        }
        if !DaWrite::is_default(&self.code_hash) {
            self.code_hash.new_value().unwrap().encode(enc)?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let bitmap = u8::decode(dec)?;

        let balance = if bitmap & 1 != 0 {
            DaRegister::new_set(CodecU256::decode(dec)?)
        } else {
            DaRegister::new_unset()
        };

        let nonce_incr = if bitmap & 2 != 0 {
            Some(u8::decode(dec)?)
        } else {
            None
        };

        let code_hash = if bitmap & 4 != 0 {
            DaRegister::new_set(CodecB256::decode(dec)?)
        } else {
            DaRegister::new_unset()
        };

        Ok(Self {
            balance,
            nonce_incr,
            code_hash,
        })
    }
}

// ============================================================================
// Account Change Type
// ============================================================================

/// Represents the type of change to an account.
#[derive(Clone, Debug)]
pub enum DaAccountChange {
    /// Account was created (new account).
    Created(DaAccountDiff),
    /// Account was updated (existing account modified).
    Updated(DaAccountDiff),
    /// Account was deleted (selfdestructed).
    Deleted,
}

/// Serde-friendly representation of DaAccountChange for RPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DaAccountChangeSerde {
    Created(DaAccountDiffSerde),
    Updated(DaAccountDiffSerde),
    Deleted,
}

impl From<&DaAccountChange> for DaAccountChangeSerde {
    fn from(change: &DaAccountChange) -> Self {
        match change {
            DaAccountChange::Created(diff) => Self::Created(diff.into()),
            DaAccountChange::Updated(diff) => Self::Updated(diff.into()),
            DaAccountChange::Deleted => Self::Deleted,
        }
    }
}

impl From<DaAccountChangeSerde> for DaAccountChange {
    fn from(serde: DaAccountChangeSerde) -> Self {
        match serde {
            DaAccountChangeSerde::Created(diff) => Self::Created(diff.into()),
            DaAccountChangeSerde::Updated(diff) => Self::Updated(diff.into()),
            DaAccountChangeSerde::Deleted => Self::Deleted,
        }
    }
}

impl DaAccountChange {
    /// Returns true if this is an empty/no-op change.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Updated(diff) => diff.is_unchanged(),
            _ => false,
        }
    }
}

impl Codec for DaAccountChange {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match self {
            Self::Created(diff) => {
                0u8.encode(enc)?;
                diff.encode(enc)?;
            }
            Self::Updated(diff) => {
                1u8.encode(enc)?;
                diff.encode(enc)?;
            }
            Self::Deleted => {
                2u8.encode(enc)?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let tag = u8::decode(dec)?;
        match tag {
            0 => Ok(Self::Created(DaAccountDiff::decode(dec)?)),
            1 => Ok(Self::Updated(DaAccountDiff::decode(dec)?)),
            2 => Ok(Self::Deleted),
            _ => Err(CodecError::InvalidVariant("DaAccountChange")),
        }
    }
}

// ============================================================================
// Storage Slot Diff
// ============================================================================

/// Diff for storage slots of an account.
///
/// Uses a sorted map for deterministic encoding.
/// Each slot value is encoded as a register (full replacement).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaStorageDiff {
    /// Changed storage slots: slot_key -> new_value (None = deleted/zeroed).
    slots: BTreeMap<U256, Option<U256>>,
}

impl DaStorageDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a slot value.
    pub fn set_slot(&mut self, key: U256, value: U256) {
        if value.is_zero() {
            self.slots.insert(key, None);
        } else {
            self.slots.insert(key, Some(value));
        }
    }

    /// Marks a slot as deleted (zeroed).
    pub fn delete_slot(&mut self, key: U256) {
        self.slots.insert(key, None);
    }

    /// Returns true if no slot changes.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Returns the number of changed slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// Iterates over slot changes.
    pub fn iter(&self) -> impl Iterator<Item = (&U256, &Option<U256>)> {
        self.slots.iter()
    }
}

impl Codec for DaStorageDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode count as varint (u32 should be enough)
        (self.slots.len() as u32).encode(enc)?;

        // Encode each slot (already sorted due to BTreeMap)
        for (key, value) in &self.slots {
            enc.write_buf(&key.to_le_bytes::<32>())?;
            match value {
                Some(v) => {
                    true.encode(enc)?;
                    enc.write_buf(&v.to_le_bytes::<32>())?;
                }
                None => {
                    false.encode(enc)?;
                }
            }
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let count = u32::decode(dec)? as usize;
        let mut slots = BTreeMap::new();

        for _ in 0..count {
            let mut key_buf = [0u8; 32];
            dec.read_buf(&mut key_buf)?;
            let key = U256::from_le_bytes(key_buf);

            let has_value = bool::decode(dec)?;
            let value = if has_value {
                let mut value_buf = [0u8; 32];
                dec.read_buf(&mut value_buf)?;
                Some(U256::from_le_bytes(value_buf))
            } else {
                None
            };

            slots.insert(key, value);
        }

        Ok(Self { slots })
    }
}

// ============================================================================
// Full EE State Diff
// ============================================================================

/// Complete EE state diff for a batch, using DA framework types.
///
/// This is the DA-optimized replacement for `BatchStateDiff`.
#[derive(Clone, Debug, Default)]
pub struct DaEeStateDiff {
    /// Account changes, sorted by address for deterministic encoding.
    pub accounts: BTreeMap<Address, DaAccountChange>,
    /// Storage slot changes per account, sorted by address.
    pub storage: BTreeMap<Address, DaStorageDiff>,
    /// Code hashes of deployed contracts (deduplicated).
    /// Full bytecode can be fetched from DB using these hashes.
    pub deployed_code_hashes: Vec<B256>,
}

/// Serde-friendly representation of DaEeStateDiff for RPC.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DaEeStateDiffSerde {
    /// Account changes, sorted by address.
    pub accounts: BTreeMap<Address, DaAccountChangeSerde>,
    /// Storage slot changes per account.
    pub storage: BTreeMap<Address, DaStorageDiff>,
    /// Code hashes of deployed contracts.
    pub deployed_code_hashes: Vec<B256>,
}

impl From<&DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: &DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage.clone(),
            deployed_code_hashes: diff.deployed_code_hashes.clone(),
        }
    }
}

impl From<DaEeStateDiff> for DaEeStateDiffSerde {
    fn from(diff: DaEeStateDiff) -> Self {
        Self {
            accounts: diff.accounts.iter().map(|(k, v)| (*k, v.into())).collect(),
            storage: diff.storage,
            deployed_code_hashes: diff.deployed_code_hashes,
        }
    }
}

impl From<DaEeStateDiffSerde> for DaEeStateDiff {
    fn from(serde: DaEeStateDiffSerde) -> Self {
        Self {
            accounts: serde
                .accounts
                .into_iter()
                .map(|(k, v)| (k, v.into()))
                .collect(),
            storage: serde.storage,
            deployed_code_hashes: serde.deployed_code_hashes,
        }
    }
}

impl DaEeStateDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if the diff is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.storage.is_empty() && self.deployed_code_hashes.is_empty()
    }

    /// Merges another diff into this one.
    ///
    /// Later changes override earlier ones. Used for batch aggregation.
    pub fn merge(&mut self, other: &DaEeStateDiff) {
        // Merge accounts - later changes override
        for (addr, change) in &other.accounts {
            self.accounts.insert(*addr, change.clone());
        }

        // Merge storage - later slot values override
        for (addr, other_storage) in &other.storage {
            let storage = self.storage.entry(*addr).or_default();
            for (key, value) in other_storage.iter() {
                if let Some(v) = value {
                    storage.set_slot(*key, *v);
                } else {
                    storage.delete_slot(*key);
                }
            }
        }

        // Merge deployed code hashes (deduplicate)
        for hash in &other.deployed_code_hashes {
            if !self.deployed_code_hashes.contains(hash) {
                self.deployed_code_hashes.push(*hash);
            }
        }
    }
}

impl Codec for DaEeStateDiff {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode accounts (sorted by BTreeMap)
        (self.accounts.len() as u32).encode(enc)?;
        for (addr, change) in &self.accounts {
            enc.write_buf(addr.as_slice())?;
            change.encode(enc)?;
        }

        // Encode storage (sorted by BTreeMap)
        (self.storage.len() as u32).encode(enc)?;
        for (addr, storage_diff) in &self.storage {
            enc.write_buf(addr.as_slice())?;
            storage_diff.encode(enc)?;
        }

        // Encode deployed code hashes
        (self.deployed_code_hashes.len() as u32).encode(enc)?;
        for hash in &self.deployed_code_hashes {
            enc.write_buf(hash.as_slice())?;
        }

        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Decode accounts
        let accounts_count = u32::decode(dec)? as usize;
        let mut accounts = BTreeMap::new();
        for _ in 0..accounts_count {
            let mut addr_buf = [0u8; 20];
            dec.read_buf(&mut addr_buf)?;
            let addr = Address::from(addr_buf);
            let change = DaAccountChange::decode(dec)?;
            accounts.insert(addr, change);
        }

        // Decode storage
        let storage_count = u32::decode(dec)? as usize;
        let mut storage = BTreeMap::new();
        for _ in 0..storage_count {
            let mut addr_buf = [0u8; 20];
            dec.read_buf(&mut addr_buf)?;
            let addr = Address::from(addr_buf);
            let storage_diff = DaStorageDiff::decode(dec)?;
            storage.insert(addr, storage_diff);
        }

        // Decode deployed code hashes
        let code_count = u32::decode(dec)? as usize;
        let mut deployed_code_hashes = Vec::with_capacity(code_count);
        for _ in 0..code_count {
            let mut hash_buf = [0u8; 32];
            dec.read_buf(&mut hash_buf)?;
            deployed_code_hashes.push(B256::from(hash_buf));
        }

        Ok(Self {
            accounts,
            storage,
            deployed_code_hashes,
        })
    }
}

// ============================================================================
// Direct Builder from BundleState
// ============================================================================

/// Builder for `DaEeStateDiff` that works directly with `BundleState`.
///
/// This builder aggregates state changes across multiple blocks, tracking original
/// values to correctly handle:
/// - Created vs Updated distinction (was account None before batch?)
/// - Revert detection (value changed back to original within batch)
/// - Proper nonce delta computation
#[derive(Clone, Debug, Default)]
pub struct DaEeStateDiffBuilder {
    /// Current account states: address -> (original_before_batch, current_state)
    /// Original is None if account didn't exist before the batch started.
    accounts: BTreeMap<Address, (Option<AccountSnapshot>, Option<AccountSnapshot>)>,

    /// Current storage states: address -> slot -> (original_value, current_value)
    storage: BTreeMap<Address, BTreeMap<U256, (U256, U256)>>,

    /// Deployed contract code hashes (deduplicated).
    deployed_code_hashes: Vec<B256>,
}

/// Snapshot of account state for tracking changes.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountSnapshot {
    balance: U256,
    nonce: u64,
    code_hash: B256,
}

impl AccountSnapshot {
    fn from_bundle_account(acc: &BundleAccount) -> Option<Self> {
        acc.info.as_ref().map(|info| Self {
            balance: info.balance,
            nonce: info.nonce,
            code_hash: info.code_hash,
        })
    }
}

impl DaEeStateDiffBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a `BundleState` from a single block.
    ///
    /// Can be called multiple times to aggregate changes across blocks.
    pub fn apply_bundle(&mut self, bundle: &BundleState) {
        // Process account changes
        for (addr, bundle_acc) in &bundle.state {
            let original_snapshot = bundle_acc.original_info.as_ref().map(|info| AccountSnapshot {
                balance: info.balance,
                nonce: info.nonce,
                code_hash: info.code_hash,
            });
            let current_snapshot = AccountSnapshot::from_bundle_account(bundle_acc);

            // Get or initialize the tracking entry
            let entry = self.accounts.entry(*addr).or_insert_with(|| {
                // First time seeing this account in the batch - record original
                (original_snapshot.clone(), None)
            });

            // Update current state
            entry.1 = current_snapshot;

            // Process storage changes for this account
            for (slot_key, slot) in &bundle_acc.storage {
                let storage_entry = self.storage.entry(*addr).or_default();
                let slot_entry = storage_entry.entry(*slot_key).or_insert_with(|| {
                    // First time seeing this slot - record original
                    (slot.previous_or_original_value, slot.previous_or_original_value)
                });
                // Update current value
                slot_entry.1 = slot.present_value;
            }
        }

        // Collect deployed contract code hashes
        for bytecode in bundle.contracts.values() {
            let code_hash = bytecode.hash_slow();
            if code_hash != KECCAK_EMPTY && !self.deployed_code_hashes.contains(&code_hash) {
                self.deployed_code_hashes.push(code_hash);
            }
        }
    }

    /// Builds the final `DaEeStateDiff`.
    ///
    /// Filters out accounts/slots that reverted to their original values.
    pub fn build(self) -> DaEeStateDiff {
        let mut result = DaEeStateDiff::new();

        // Process accounts
        for (addr, (original, current)) in self.accounts {
            // Skip if current equals original (reverted within batch)
            if original == current {
                continue;
            }

            let change = match (original, current) {
                (None, Some(curr)) => {
                    // Account was created
                    DaAccountChange::Created(DaAccountDiff {
                        balance: DaRegister::new_set(CodecU256(curr.balance)),
                        nonce_incr: if curr.nonce > 0 {
                            Some(curr.nonce.min(u8::MAX as u64) as u8)
                        } else {
                            None
                        },
                        code_hash: if curr.code_hash != KECCAK_EMPTY {
                            DaRegister::new_set(CodecB256(curr.code_hash))
                        } else {
                            DaRegister::new_unset()
                        },
                    })
                }
                (Some(orig), Some(curr)) => {
                    // Account was updated
                    let balance = if orig.balance != curr.balance {
                        DaRegister::new_set(CodecU256(curr.balance))
                    } else {
                        DaRegister::new_unset()
                    };

                    let nonce_delta = curr.nonce.saturating_sub(orig.nonce);
                    let nonce_incr = if nonce_delta > 0 {
                        Some(nonce_delta.min(u8::MAX as u64) as u8)
                    } else {
                        None
                    };

                    let code_hash = if orig.code_hash != curr.code_hash {
                        DaRegister::new_set(CodecB256(curr.code_hash))
                    } else {
                        DaRegister::new_unset()
                    };

                    let diff = DaAccountDiff {
                        balance,
                        nonce_incr,
                        code_hash,
                    };

                    // Skip if no actual changes
                    if diff.is_unchanged() {
                        continue;
                    }

                    DaAccountChange::Updated(diff)
                }
                (Some(_), None) => {
                    // Account was deleted
                    DaAccountChange::Deleted
                }
                (None, None) => {
                    // No change (shouldn't happen, but handle gracefully)
                    continue;
                }
            };

            result.accounts.insert(addr, change);
        }

        // Process storage
        for (addr, slots) in self.storage {
            let mut storage_diff = DaStorageDiff::new();

            for (key, (original, current)) in slots {
                // Skip if reverted to original
                if original == current {
                    continue;
                }

                if current.is_zero() {
                    storage_diff.delete_slot(key);
                } else {
                    storage_diff.set_slot(key, current);
                }
            }

            if !storage_diff.is_empty() {
                result.storage.insert(addr, storage_diff);
            }
        }

        result.deployed_code_hashes = self.deployed_code_hashes;
        result
    }
}

// ============================================================================
// Conversion from BundleState (single block)
// ============================================================================

impl From<&BundleState> for DaEeStateDiff {
    fn from(bundle: &BundleState) -> Self {
        let mut builder = DaEeStateDiffBuilder::new();
        builder.apply_bundle(bundle);
        builder.build()
    }
}

impl From<BundleState> for DaEeStateDiff {
    fn from(bundle: BundleState) -> Self {
        Self::from(&bundle)
    }
}

// ============================================================================
// Conversion from BatchStateDiff (legacy support)
// ============================================================================

impl From<BatchStateDiff> for DaEeStateDiff {
    fn from(batch: BatchStateDiff) -> Self {
        let mut result = DaEeStateDiff::new();

        // Convert accounts
        for (addr, account_opt) in batch.accounts {
            let change = match account_opt {
                Some(account) => {
                    // Account exists - treat as "created" with full values
                    // (since BatchStateDiff doesn't track original values,
                    // we can't determine if it was created or updated)
                    let diff = DaAccountDiff {
                        balance: DaRegister::new_set(CodecU256(account.balance)),
                        nonce_incr: if account.nonce > 0 {
                            // Clamp to u8 max
                            Some(account.nonce.min(u8::MAX as u64) as u8)
                        } else {
                            None
                        },
                        code_hash: if account.code_hash != KECCAK_EMPTY {
                            DaRegister::new_set(CodecB256(account.code_hash))
                        } else {
                            DaRegister::new_unset()
                        },
                    };
                    DaAccountChange::Created(diff)
                }
                None => DaAccountChange::Deleted,
            };
            result.accounts.insert(addr, change);
        }

        // Convert storage slots
        for (addr, slots) in batch.storage_slots {
            let mut storage_diff = DaStorageDiff::new();
            for (key, value) in slots {
                if value.is_zero() {
                    storage_diff.delete_slot(key);
                } else {
                    storage_diff.set_slot(key, value);
                }
            }
            if !storage_diff.is_empty() {
                result.storage.insert(addr, storage_diff);
            }
        }

        // Convert contracts to code hashes only (deduplication)
        for bytecode in batch.contracts {
            let code_hash = bytecode.hash_slow();
            if !result.deployed_code_hashes.contains(&code_hash) {
                result.deployed_code_hashes.push(code_hash);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_codec_u256_roundtrip() {
        let val = CodecU256(U256::from(0x1234567890abcdefu64));
        let encoded = encode_to_vec(&val).unwrap();
        let decoded: CodecU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_codec_b256_roundtrip() {
        let val = CodecB256(B256::from([0x42u8; 32]));
        let encoded = encode_to_vec(&val).unwrap();
        let decoded: CodecB256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_account_diff_unchanged() {
        let diff = DaAccountDiff::new_unchanged();
        assert!(diff.is_unchanged());

        let encoded = encode_to_vec(&diff).unwrap();
        // Should just be 1 byte (bitmap = 0)
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0], 0);

        let decoded: DaAccountDiff = decode_buf_exact(&encoded).unwrap();
        assert!(decoded.is_unchanged());
    }

    #[test]
    fn test_account_diff_created() {
        let diff = DaAccountDiff::new_created(U256::from(1000), 1, B256::from([0x11u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaAccountDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.balance.new_value().unwrap().0, U256::from(1000));
        assert_eq!(decoded.nonce_incr, Some(1u8));
        assert_eq!(
            decoded.code_hash.new_value().unwrap().0,
            B256::from([0x11u8; 32])
        );
    }

    #[test]
    fn test_account_diff_from_change() {
        let original = DaAccountState::new(U256::from(1000), 5, B256::from([0x11u8; 32]));
        let new = DaAccountState::new(
            U256::from(2000),
            7,                        // +2 increment
            B256::from([0x11u8; 32]), // unchanged
        );

        let diff = DaAccountDiff::from_change(&original, &new).unwrap();

        // Balance changed
        assert!(!DaWrite::is_default(&diff.balance));
        assert_eq!(diff.balance.new_value().unwrap().0, U256::from(2000));

        // Nonce incremented by 2
        assert_eq!(diff.nonce_incr, Some(2u8));

        // Code hash unchanged
        assert!(DaWrite::is_default(&diff.code_hash));
    }

    #[test]
    fn test_account_diff_apply() {
        let mut state = DaAccountState::new(U256::from(1000), 5, B256::from([0x11u8; 32]));

        let diff = DaAccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(2000))),
            nonce_incr: Some(3),
            code_hash: DaRegister::new_unset(),
        };

        ContextlessDaWrite::apply(&diff, &mut state).unwrap();

        assert_eq!(state.balance.0, U256::from(2000));
        assert_eq!(state.nonce, 8); // 5 + 3
        assert_eq!(state.code_hash.0, B256::from([0x11u8; 32])); // unchanged
    }

    #[test]
    fn test_storage_diff_roundtrip() {
        let mut diff = DaStorageDiff::new();
        diff.set_slot(U256::from(1), U256::from(100));
        diff.set_slot(U256::from(2), U256::from(200));
        diff.delete_slot(U256::from(3));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaStorageDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.len(), 3);
        assert_eq!(
            decoded.slots.get(&U256::from(1)),
            Some(&Some(U256::from(100)))
        );
        assert_eq!(
            decoded.slots.get(&U256::from(2)),
            Some(&Some(U256::from(200)))
        );
        assert_eq!(decoded.slots.get(&U256::from(3)), Some(&None));
    }

    #[test]
    fn test_account_change_roundtrip() {
        let created =
            DaAccountChange::Created(DaAccountDiff::new_created(U256::from(1000), 1, B256::ZERO));
        let updated = DaAccountChange::Updated(DaAccountDiff {
            balance: DaRegister::new_set(CodecU256(U256::from(500))),
            nonce_incr: None,
            code_hash: DaRegister::new_unset(),
        });
        let deleted = DaAccountChange::Deleted;

        for change in [created, updated, deleted] {
            let encoded = encode_to_vec(&change).unwrap();
            let decoded: DaAccountChange = decode_buf_exact(&encoded).unwrap();

            // Verify tag matches
            match (&change, &decoded) {
                (DaAccountChange::Created(_), DaAccountChange::Created(_)) => {}
                (DaAccountChange::Updated(_), DaAccountChange::Updated(_)) => {}
                (DaAccountChange::Deleted, DaAccountChange::Deleted) => {}
                _ => panic!("Tag mismatch"),
            }
        }
    }

    #[test]
    fn test_ee_state_diff_roundtrip() {
        let mut diff = DaEeStateDiff::new();

        // Add account change
        diff.accounts.insert(
            Address::from([0x11u8; 20]),
            DaAccountChange::Created(DaAccountDiff::new_created(
                U256::from(1000),
                1,
                B256::from([0x22u8; 32]),
            )),
        );

        // Add storage change
        let mut storage = DaStorageDiff::new();
        storage.set_slot(U256::from(1), U256::from(100));
        diff.storage.insert(Address::from([0x11u8; 20]), storage);

        // Add deployed code hash
        diff.deployed_code_hashes.push(B256::from([0x33u8; 32]));

        let encoded = encode_to_vec(&diff).unwrap();
        let decoded: DaEeStateDiff = decode_buf_exact(&encoded).unwrap();

        assert_eq!(decoded.accounts.len(), 1);
        assert_eq!(decoded.storage.len(), 1);
        assert_eq!(decoded.deployed_code_hashes.len(), 1);
        assert_eq!(decoded.deployed_code_hashes[0], B256::from([0x33u8; 32]));
    }

    #[test]
    fn test_empty_diff_size() {
        let diff = DaEeStateDiff::new();
        let encoded = encode_to_vec(&diff).unwrap();
        // Should be minimal: 3 u32 counts (0, 0, 0) = 3 bytes minimum
        assert!(encoded.len() <= 12);
    }

    #[test]
    fn test_conversion_from_batch_state_diff() {
        use std::collections::HashSet;

        use revm::state::Bytecode;
        use revm_primitives::HashMap;

        use crate::account::Account;

        // Create a BatchStateDiff
        let mut accounts = HashMap::default();
        accounts.insert(
            Address::from([0x11u8; 20]),
            Some(Account {
                balance: U256::from(1000),
                nonce: 5,
                code_hash: B256::from([0x22u8; 32]),
            }),
        );
        accounts.insert(Address::from([0x33u8; 20]), None); // Deleted account

        let mut storage_slots = HashMap::default();
        let mut slots = HashMap::default();
        slots.insert(U256::from(1), U256::from(100));
        slots.insert(U256::from(2), U256::ZERO); // Deleted slot
        storage_slots.insert(Address::from([0x11u8; 20]), slots);

        let mut contracts = HashSet::default();
        contracts.insert(Bytecode::new_legacy(vec![0x60, 0x00].into()));

        let batch = BatchStateDiff {
            accounts,
            contracts,
            storage_slots,
        };

        // Convert to DaEeStateDiff
        let da_diff: DaEeStateDiff = batch.into();

        // Verify accounts
        assert_eq!(da_diff.accounts.len(), 2);
        assert!(matches!(
            da_diff.accounts.get(&Address::from([0x11u8; 20])),
            Some(DaAccountChange::Created(_))
        ));
        assert!(matches!(
            da_diff.accounts.get(&Address::from([0x33u8; 20])),
            Some(DaAccountChange::Deleted)
        ));

        // Verify storage
        assert_eq!(da_diff.storage.len(), 1);
        let storage = da_diff.storage.get(&Address::from([0x11u8; 20])).unwrap();
        assert_eq!(storage.len(), 2);

        // Verify deployed code hashes (deduplicated)
        assert_eq!(da_diff.deployed_code_hashes.len(), 1);

        // Verify roundtrip encoding
        let encoded = encode_to_vec(&da_diff).unwrap();
        let decoded: DaEeStateDiff = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.accounts.len(), da_diff.accounts.len());
        assert_eq!(decoded.storage.len(), da_diff.storage.len());
        assert_eq!(
            decoded.deployed_code_hashes.len(),
            da_diff.deployed_code_hashes.len()
        );
    }
}
