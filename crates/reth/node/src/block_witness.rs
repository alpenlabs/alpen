//! Inline per-block proof-witness capture, produced during payload build.
//!
//! Harvests the raw execution-witness parts for a freshly built block by
//! reading the access set straight out of the just-executed reth [`State`] — no
//! re-execution. This is the producer side of the witness path: capture happens
//! in `try_build_payload` (see [`crate::payload_builder`]) while the block is at
//! tip.
//!
//! Each block's [`BlockWitnessRecord`] stores only the *raw* witness inputs (the
//! trie-node bag, loaded bytecodes, and BLOCKHASH ancestor headers), not a
//! built trie. The chunk prover unions these per-block bags into one chunk-level
//! sparse state at assembly time (see `EvmPartialState::from_witness_parts`), so
//! the trie reconstruction is a chunk-level concern.

use std::collections::BTreeMap;

use alloy_consensus::Header;
use alloy_primitives::{keccak256, Bytes, B256};
use reth_provider::{AccountReader, BytecodeReader, HeaderProvider, StateProofProvider};
use reth_revm::{db::State, witness::ExecutionWitnessRecord, Database};
use reth_trie::TrieInput;
use revm_primitives::KECCAK_EMPTY;
use serde::{Deserialize, Serialize};

/// Persisted per-block proof-witness, keyed by execution block hash.
///
/// Holds the raw witness inputs the chunk prover needs to reconstruct state: the
/// trie-node bag (`witness_state`), the bytecodes the block loaded (`codes`),
/// and the BLOCKHASH ancestor headers — plus the RLP block and parent header.
/// The chunk prover unions the node bags across a chunk's blocks and builds one
/// chunk-level sparse state from them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockWitnessRecord {
    /// Bag of RLP-encoded MPT nodes for the block's touched paths, anchored at
    /// the block's parent state root (reth's `ExecutionWitness::state` format).
    pub witness_state: Vec<Vec<u8>>,
    /// Bytecodes the block loaded (raw bytes; keyed by keccak hash downstream).
    pub codes: Vec<Vec<u8>>,
    /// RLP-encoded BLOCKHASH ancestor headers covering the block's range.
    pub ancestor_headers: Vec<Vec<u8>>,
    /// RLP-encoded reth `Block` (header + body) for guest re-execution.
    pub raw_block_rlp: Vec<u8>,
    /// RLP-encoded parent alloy [`Header`] (anchors the block's pre-state root).
    /// For a chunk's first block this is the chunk's `prev_header`.
    pub raw_parent_header_rlp: Vec<u8>,
}

impl BlockWitnessRecord {
    /// Encodes the record to CBOR for storage.
    pub fn encode(&self) -> eyre::Result<Vec<u8>> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf)
            .map_err(|e| eyre::eyre!("cbor encode block witness record: {e}"))?;
        Ok(buf)
    }

    /// Decodes a CBOR-encoded record from storage.
    pub fn decode(bytes: &[u8]) -> eyre::Result<Self> {
        ciborium::from_reader(bytes)
            .map_err(|e| eyre::eyre!("cbor decode block witness record: {e}"))
    }
}

/// Harvests the raw depth-0 witness parts from an already-executed block state,
/// with **no re-execution**.
///
/// Reads the access set directly out of the `executed_state` produced while the
/// block was built — the live reth [`State`] right after `BlockBuilder::finish`.
/// Reusing that state is the whole point of inline capture: the single
/// production execution both commits state and yields its access set.
///
/// `executed_state` supplies the access set (touched accounts/slots, loaded
/// `codes`, BLOCKHASH range) via reth's [`ExecutionWitnessRecord`].
/// `state_provider` must be the parent state (it serves the depth-0
/// [`StateProofProvider::witness`] trie nodes and the pre-state code of
/// accessed accounts), and `header_provider` must cover the BLOCKHASH
/// ancestor range.
pub fn build_block_witness_from_executed_state<DB, SP, HP>(
    executed_state: &State<DB>,
    state_provider: &SP,
    header_provider: &HP,
    block_num: u64,
    block_rlp: Vec<u8>,
    parent_header: &Header,
) -> eyre::Result<BlockWitnessRecord>
where
    DB: Database,
    SP: StateProofProvider + AccountReader + BytecodeReader,
    HP: HeaderProvider<Header = Header>,
{
    // Access set read straight out of the post-execution state — no re-run.
    let mut record = ExecutionWitnessRecord::default();
    record.record_executed_state(executed_state);
    let ExecutionWitnessRecord {
        hashed_state,
        codes,
        lowest_block_number,
        ..
    } = record;

    // Trie nodes covering the block's touched paths (against the parent state).
    let witness_state = state_provider
        .witness(TrieInput::default(), hashed_state)?
        .into_iter()
        .map(|node| node.to_vec())
        .collect();

    let codes = collect_accessed_codes(executed_state, state_provider, codes)?;

    // BLOCKHASH ancestor headers: the contiguous range from the lowest block
    // referenced (or just the parent) up to the parent.
    let smallest = lowest_block_number.unwrap_or_else(|| block_num.saturating_sub(1));
    let ancestor_headers = header_provider
        .headers_range(smallest..block_num)?
        .iter()
        .map(alloy_rlp::encode)
        .collect();

    let raw_parent_header_rlp = alloy_rlp::encode(parent_header);

    Ok(BlockWitnessRecord {
        witness_state,
        codes,
        ancestor_headers,
        raw_block_rlp: block_rlp,
        raw_parent_header_rlp,
    })
}

/// Collects every bytecode the block accessed, deduped by code hash, to store
/// in the block witness.
///
/// The witness is the only source of bytecode when the block is later
/// re-executed for proving, so it must carry every code a cold replay of the
/// block loads. reth's [`ExecutionWitnessRecord`] is not enough on its own: it
/// reports only code that passed through its in-memory bytecode stores
/// (`cache.contracts` + `bundle_state.contracts`), so a contract whose code
/// reached the EVM solely via its account's `info.code` — a warm load that
/// never issued a by-hash fetch — is silently left out.
///
/// This starts from reth's `record_codes` and supplements it twice:
/// - the code of every accessed account that still carries one post-block (covers warm-attached
///   loads of unchanged code);
/// - the **pre-state** code of every accessed account, fetched from the parent `state_provider`.
///   This covers code that the block read and then replaced or cleared (today only EIP-7702
///   re-delegation/clearing): the old code can arrive warm-attached (e.g. via the payload builder's
///   pre-warmed `CachedReads` from the parent block), so it never hits `code_by_hash`, and
///   post-block the account carries only the new code — leaving the old one in neither store. A
///   cold replay of the block still reads it during authorization validation, so the witness must
///   include it.
fn collect_accessed_codes<DB, SP>(
    executed_state: &State<DB>,
    state_provider: &SP,
    record_codes: Vec<Bytes>,
) -> eyre::Result<Vec<Vec<u8>>>
where
    DB: Database,
    SP: AccountReader + BytecodeReader,
{
    let mut codes_by_hash: BTreeMap<B256, Bytes> = record_codes
        .into_iter()
        .map(|code| (keccak256(&code), code))
        .collect();

    for account in executed_state.cache.accounts.values() {
        let Some(plain) = &account.account else {
            continue;
        };
        if plain.info.is_empty_code_hash() {
            continue;
        }
        let Some(code) = &plain.info.code else {
            continue;
        };

        // The guest content-addresses code by `keccak256(bytes)`, so code stored
        // under a hash that disagrees with its bytes would be unreachable there.
        // Surface the inconsistency as a build failure now, not a guest panic.
        let actual_hash = code.hash_slow();
        if actual_hash != plain.info.code_hash {
            eyre::bail!(
                "accessed account bytecode hash mismatch: expected_code_hash={}, actual_hash={actual_hash}",
                plain.info.code_hash,
            );
        }

        codes_by_hash
            .entry(plain.info.code_hash)
            .or_insert_with(|| code.original_bytes());
    }

    // Pre-state code of every accessed account. The post-block sweep above sees
    // only the code an account ends the block with, so code replaced or cleared
    // within the block is recoverable solely from the parent state.
    for address in executed_state.cache.accounts.keys() {
        let Some(pre_account) = state_provider.basic_account(address)? else {
            continue;
        };
        let Some(pre_code_hash) = pre_account.bytecode_hash else {
            continue;
        };
        if pre_code_hash == KECCAK_EMPTY || codes_by_hash.contains_key(&pre_code_hash) {
            continue;
        }

        let code = state_provider.bytecode_by_hash(&pre_code_hash)?.ok_or_else(|| {
            eyre::eyre!(
                "pre-state bytecode missing from provider: account={address}, code_hash={pre_code_hash}",
            )
        })?;
        let bytes = code.original_bytes();

        // Same guest-reachability guarantee as above: the guest keys code by
        // `keccak256(bytes)`, so a store hash that disagrees with the bytes
        // must fail the build, not the proof.
        let actual_hash = keccak256(&bytes);
        if actual_hash != pre_code_hash {
            eyre::bail!(
                "pre-state bytecode hash mismatch: account={address}, expected_code_hash={pre_code_hash}, actual_hash={actual_hash}",
            );
        }

        codes_by_hash.insert(pre_code_hash, bytes);
    }

    Ok(codes_by_hash
        .into_values()
        .map(|code| code.to_vec())
        .collect())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_primitives::{Address, U256};
    use reth_errors::ProviderResult;
    use reth_primitives_traits::{Account, Bytecode as RethBytecode};
    use reth_revm::db::{states::cache_account::CacheAccount, EmptyDB, State};
    use revm::state::{AccountInfo, Bytecode};

    use super::*;

    /// Parent-state stub serving only the two lookups the pre-state code fetch
    /// needs.
    #[derive(Default)]
    struct StubStateProvider {
        accounts: HashMap<Address, Account>,
        codes: HashMap<B256, Bytecode>,
    }

    impl StubStateProvider {
        fn with_account_code(address: Address, code: Bytecode) -> Self {
            let code_hash = code.hash_slow();
            Self {
                accounts: HashMap::from([(
                    address,
                    Account {
                        nonce: 1,
                        balance: U256::ZERO,
                        bytecode_hash: Some(code_hash),
                    },
                )]),
                codes: HashMap::from([(code_hash, code)]),
            }
        }
    }

    impl AccountReader for StubStateProvider {
        fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
            Ok(self.accounts.get(address).copied())
        }
    }

    impl BytecodeReader for StubStateProvider {
        fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<RethBytecode>> {
            Ok(self.codes.get(code_hash).cloned().map(RethBytecode))
        }
    }

    /// A 23-byte EIP-7702 delegation designator (`0xef0100 || address`).
    fn delegation_designator(target: u8) -> Bytecode {
        let mut raw = vec![0xef, 0x01, 0x00];
        raw.extend_from_slice(&[target; 20]);
        Bytecode::new_raw(Bytes::from(raw))
    }

    /// A contract whose code is attached to its account `info.code` but never
    /// entered `cache.contracts` (the in-memory bytecode store
    /// `record_executed_state` reads) must still be captured: otherwise the guest
    /// panics in `WitnessDB::code_by_hash_ref` when that bytecode is missing.
    #[test]
    fn collects_code_attached_to_account_info_not_in_dedup_map() {
        // A pre-existing contract whose code only lives on the loaded account.
        let raw = Bytes::from_static(&[0x60, 0x00, 0x60, 0x00, 0xf3]); // PUSH1 0 PUSH1 0 RETURN
        let code = Bytecode::new_raw(raw);
        let code_hash = code.hash_slow();

        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: Some(code),
        };
        state.cache.accounts.insert(
            Address::repeat_byte(0x42),
            CacheAccount::new_loaded(info, Default::default()),
        );

        // `record_executed_state` produced no codes (dedup map was empty), the
        // exact condition that dropped the bytecode before the fix.
        let codes =
            collect_accessed_codes(&state, &StubStateProvider::default(), Vec::new()).unwrap();

        assert!(
            codes.iter().any(|c| keccak256(c) == code_hash),
            "accessed contract code must be captured even when only on info.code"
        );
    }

    /// An empty-code (EOA) account must not contribute a bytecode entry.
    #[test]
    fn skips_accounts_without_code() {
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::from(5u64),
            nonce: 0,
            ..Default::default()
        };
        state.cache.accounts.insert(
            Address::repeat_byte(0x7),
            CacheAccount::new_loaded(info, Default::default()),
        );

        assert!(
            collect_accessed_codes(&state, &StubStateProvider::default(), Vec::new())
                .unwrap()
                .is_empty()
        );
    }

    /// Locks the exact bug shape: reth's raw `ExecutionWitnessRecord` can miss
    /// code attached to an accessed account, but Alpen's block witness producer
    /// must include it before persisting the record.
    #[test]
    fn regression_supplements_codes_missing_from_reth_record() {
        let raw = Bytes::from_static(&[0x60, 0x2a, 0x60, 0x00, 0x52]);
        let code = Bytecode::new_raw(raw);
        let code_hash = code.hash_slow();

        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: Some(code),
        };
        state.cache.accounts.insert(
            Address::repeat_byte(0x24),
            CacheAccount::new_loaded(info, Default::default()),
        );

        let mut record = ExecutionWitnessRecord::default();
        record.record_executed_state(&state);
        assert!(
            !record.codes.iter().any(|code| keccak256(code) == code_hash),
            "raw reth record should reproduce the missing-code condition"
        );

        let codes =
            collect_accessed_codes(&state, &StubStateProvider::default(), record.codes).unwrap();
        assert!(
            codes.iter().any(|code| keccak256(code) == code_hash),
            "block witness codes must include every accessed account info.code"
        );
    }

    /// Account-attached bytecode must be content-addressed by the account's
    /// code hash. Otherwise the guest would still miss it after rehashing.
    #[test]
    fn rejects_account_info_code_with_mismatched_hash() {
        let raw = Bytes::from_static(&[0x60, 0x00, 0x56]);
        let code = Bytecode::new_raw(raw);
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash: B256::repeat_byte(0x11),
            code: Some(code),
        };
        state.cache.accounts.insert(
            Address::repeat_byte(0x55),
            CacheAccount::new_loaded(info, Default::default()),
        );

        let err =
            collect_accessed_codes(&state, &StubStateProvider::default(), Vec::new()).unwrap_err();
        assert!(
            err.to_string().contains("bytecode hash mismatch"),
            "unexpected error: {err}"
        );
    }

    /// A code supplied by both reth's record and an accessed account's
    /// `info.code` is captured exactly once; supplementing is idempotent.
    #[test]
    fn dedupes_code_present_in_both_record_and_account_info() {
        let raw = Bytes::from_static(&[0x60, 0x01, 0x60, 0x02, 0x01]); // PUSH1 1 PUSH1 2 ADD
        let code = Bytecode::new_raw(raw.clone());
        let code_hash = code.hash_slow();

        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: Some(code),
        };
        state.cache.accounts.insert(
            Address::repeat_byte(0x33),
            CacheAccount::new_loaded(info, Default::default()),
        );

        // The same bytecode arrives via reth's record_codes and the account.
        let codes =
            collect_accessed_codes(&state, &StubStateProvider::default(), vec![raw]).unwrap();

        assert_eq!(codes.len(), 1, "duplicate code must collapse to one entry");
        assert_eq!(keccak256(&codes[0]), code_hash);
    }

    /// Locks the cross-chunk EIP-7702 bug shape: the block *clears* a
    /// delegation whose designator arrived warm-attached (never through
    /// `code_by_hash`, so reth's record misses it) and post-block the account
    /// carries empty code (so the attached-code sweep misses it too). The
    /// pre-state fetch must recover the old designator from the parent state,
    /// or guest replay of the clearing block panics on the by-hash lookup.
    #[test]
    fn regression_fetches_prestate_code_cleared_within_the_block() {
        let address = Address::repeat_byte(0xaa);
        let old_designator = delegation_designator(0x77);
        let old_code_hash = old_designator.hash_slow();
        let provider = StubStateProvider::with_account_code(address, old_designator);

        // Post-block state: delegation cleared, account holds no code.
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 2,
            ..Default::default()
        };
        state
            .cache
            .accounts
            .insert(address, CacheAccount::new_loaded(info, Default::default()));

        // Nothing passed through code_by_hash during the build (warm attach).
        let codes = collect_accessed_codes(&state, &provider, Vec::new()).unwrap();

        assert!(
            codes.iter().any(|c| keccak256(c) == old_code_hash),
            "pre-state designator must be captured when the block clears it"
        );
    }

    /// The re-delegation variant: post-block the account carries the *new*
    /// designator, which the attached-code sweep captures — but replay also
    /// reads the *old* one during authorization validation, so both must land
    /// in the witness.
    #[test]
    fn fetches_prestate_code_replaced_within_the_block() {
        let address = Address::repeat_byte(0xbb);
        let old_designator = delegation_designator(0x11);
        let old_code_hash = old_designator.hash_slow();
        let provider = StubStateProvider::with_account_code(address, old_designator);

        let new_designator = delegation_designator(0x22);
        let new_code_hash = new_designator.hash_slow();
        let info = AccountInfo {
            balance: U256::ZERO,
            nonce: 2,
            code_hash: new_code_hash,
            code: Some(new_designator),
        };
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        state
            .cache
            .accounts
            .insert(address, CacheAccount::new_loaded(info, Default::default()));

        let codes = collect_accessed_codes(&state, &provider, Vec::new()).unwrap();

        assert!(
            codes.iter().any(|c| keccak256(c) == old_code_hash),
            "old designator must come from the pre-state fetch"
        );
        assert!(
            codes.iter().any(|c| keccak256(c) == new_code_hash),
            "new designator must come from the attached-code sweep"
        );
    }

    /// Pre-state code already captured by reth's record must not be re-fetched
    /// or duplicated, and codeless pre-state accounts contribute nothing.
    #[test]
    fn prestate_fetch_dedupes_and_skips_codeless_accounts() {
        let contract = Address::repeat_byte(0xcc);
        let raw = Bytes::from_static(&[0x60, 0x2a]); // PUSH1 42
        let code = Bytecode::new_raw(raw.clone());
        let code_hash = code.hash_slow();
        let mut provider = StubStateProvider::with_account_code(contract, code);

        // An accessed EOA with no pre-state code.
        let eoa = Address::repeat_byte(0xdd);
        provider.accounts.insert(
            eoa,
            Account {
                nonce: 3,
                balance: U256::from(7u64),
                bytecode_hash: None,
            },
        );

        let mut state = State::builder().with_database(EmptyDB::default()).build();
        for address in [contract, eoa] {
            state.cache.accounts.insert(
                address,
                CacheAccount::new_loaded(AccountInfo::default(), Default::default()),
            );
        }

        // The contract's code already reached reth's record via code_by_hash.
        let codes = collect_accessed_codes(&state, &provider, vec![raw]).unwrap();

        assert_eq!(codes.len(), 1, "no duplicates, nothing for the EOA");
        assert_eq!(keccak256(&codes[0]), code_hash);
    }

    /// A pre-state code hash whose bytecode the provider cannot serve must fail
    /// the build: the witness would be incomplete and the guest would panic
    /// later at the by-hash lookup.
    #[test]
    fn bails_when_prestate_bytecode_is_missing_from_provider() {
        let address = Address::repeat_byte(0xee);
        let mut provider =
            StubStateProvider::with_account_code(address, delegation_designator(0x55));
        provider.codes.clear();

        let mut state = State::builder().with_database(EmptyDB::default()).build();
        state.cache.accounts.insert(
            address,
            CacheAccount::new_loaded(AccountInfo::default(), Default::default()),
        );

        let err = collect_accessed_codes(&state, &provider, Vec::new()).unwrap_err();
        assert!(
            err.to_string().contains("pre-state bytecode missing"),
            "unexpected error: {err}"
        );
    }
}
