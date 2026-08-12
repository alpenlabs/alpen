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
use alloy_primitives::{keccak256, Address, Bytes, B256};
use reth_provider::{BytecodeReader, HeaderProvider, StateProofProvider};
use reth_revm::{
    db::State,
    state::{AccountInfo, Bytecode},
    witness::ExecutionWitnessRecord,
    Database,
};
use reth_trie::TrieInput;
use serde::{Deserialize, Serialize};
use tracing::debug;

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
/// [`StateProofProvider::witness`] trie nodes), and `header_provider` must cover
/// the BLOCKHASH ancestor range. `authorization_targets` must contain every
/// EIP-7702 target declared by the block's signed transactions, including
/// invalid authorizations; content-addressed extras are safe, while omissions
/// can make transient delegation code unavailable during guest replay.
pub fn build_block_witness_from_executed_state<DB, SP, HP>(
    executed_state: &State<DB>,
    state_provider: &SP,
    header_provider: &HP,
    block_num: u64,
    block_rlp: Vec<u8>,
    parent_header: &Header,
    authorization_targets: impl IntoIterator<Item = Address>,
) -> eyre::Result<BlockWitnessRecord>
where
    DB: Database,
    SP: StateProofProvider + BytecodeReader,
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

    let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
        collect_accessed_codes(executed_state, codes, authorization_targets, |code_hash| {
            state_provider
                .bytecode_by_hash(code_hash)
                .map(|code| code.map(|code| code.original_bytes()))
                .map_err(|error| {
                    eyre::eyre!(
                        "read parent-state bytecode {code_hash} while closing block witness: \
                             {error}"
                    )
                })
        })?;
    debug!(
        block_num,
        authorization_bytecodes_added,
        parent_state_bytecodes_added,
        total_bytecodes = codes.len(),
        "closed block witness over execution, parent-state, and EIP-7702 bytecode"
    );

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
/// re-executed for proving, so it must carry every code the block touched.
/// reth's [`ExecutionWitnessRecord`] is not enough on its own: it reports only
/// code that passed through its in-memory bytecode stores (`cache.contracts` +
/// `bundle_state.contracts`), so a contract whose code reached the EVM solely
/// via its account's `info.code` — a warm load that never issued a by-hash
/// fetch — is silently left out.
///
/// This starts from reth's `record_codes`, adds every transient EIP-7702
/// designation declared by the block, then closes both the live cache and the
/// execution bundle over bytecode. Account entries are allowed to carry only a
/// code hash; those hashes are resolved against the block's parent-state
/// provider. The bundle's `original_info` is required for contracts replaced or
/// cleared during the block, while its final `info` covers newly installed code.
fn collect_accessed_codes<DB, F>(
    executed_state: &State<DB>,
    record_codes: Vec<Bytes>,
    authorization_targets: impl IntoIterator<Item = Address>,
    mut parent_bytecode_by_hash: F,
) -> eyre::Result<(Vec<Vec<u8>>, usize, usize)>
where
    DB: Database,
    F: FnMut(&B256) -> eyre::Result<Option<Bytes>>,
{
    let mut codes_by_hash = BTreeMap::new();

    for code in record_codes {
        let code_hash = keccak256(&code);
        insert_code(&mut codes_by_hash, code_hash, code, "execution witness")?;
    }

    // Revm synthesizes an EIP-7702 delegation designation directly from each
    // valid authorization target. A designation can then be replaced or
    // cleared in the same block, so it may be absent from both the execution
    // witness and the final account state even though guest replay still needs
    // to resolve its code hash. Include every non-reset target from the signed
    // transaction inputs. Invalid authorizations may add inert content-addressed
    // bytes; duplicating revm's validity rules here would be a second consensus
    // implementation and risks rejecting valid future inputs.
    let mut authorization_bytecodes_added = 0;
    for target in authorization_targets {
        if target.is_zero() {
            continue;
        }

        let designation = Bytecode::new_eip7702(target);
        if insert_code(
            &mut codes_by_hash,
            designation.hash_slow(),
            designation.original_bytes(),
            "EIP-7702 authorization",
        )? {
            authorization_bytecodes_added += 1;
        }
    }

    let mut parent_state_bytecodes_added = 0;
    for (address, account) in &executed_state.cache.accounts {
        let Some(plain) = &account.account else {
            continue;
        };

        if collect_account_info_code(
            &mut codes_by_hash,
            *address,
            &plain.info,
            "execution cache account",
            &mut parent_bytecode_by_hash,
        )? {
            parent_state_bytecodes_added += 1;
        }
    }

    for (address, account) in &executed_state.bundle_state.state {
        if let Some(original_info) = &account.original_info {
            if collect_account_info_code(
                &mut codes_by_hash,
                *address,
                original_info,
                "execution bundle pre-state account",
                &mut parent_bytecode_by_hash,
            )? {
                parent_state_bytecodes_added += 1;
            }
        }

        if let Some(info) = &account.info {
            if collect_account_info_code(
                &mut codes_by_hash,
                *address,
                info,
                "execution bundle post-state account",
                &mut parent_bytecode_by_hash,
            )? {
                parent_state_bytecodes_added += 1;
            }
        }
    }

    let codes = codes_by_hash
        .into_values()
        .map(|code| code.to_vec())
        .collect();
    Ok((
        codes,
        authorization_bytecodes_added,
        parent_state_bytecodes_added,
    ))
}

fn collect_account_info_code<F>(
    codes_by_hash: &mut BTreeMap<B256, Bytes>,
    address: Address,
    info: &AccountInfo,
    source: &'static str,
    parent_bytecode_by_hash: &mut F,
) -> eyre::Result<bool>
where
    F: FnMut(&B256) -> eyre::Result<Option<Bytes>>,
{
    if info.is_empty_code_hash() {
        return Ok(false);
    }

    if let Some(code) = &info.code {
        insert_code(codes_by_hash, info.code_hash, code.original_bytes(), source)?;
        return Ok(false);
    }

    if codes_by_hash.contains_key(&info.code_hash) {
        return Ok(false);
    }

    let code = parent_bytecode_by_hash(&info.code_hash)?.ok_or_else(|| {
        eyre::eyre!(
            "{source} {address} references bytecode {} but the parent-state provider returned \
             no bytes",
            info.code_hash
        )
    })?;
    insert_code(codes_by_hash, info.code_hash, code, source)?;
    Ok(true)
}

fn insert_code(
    codes_by_hash: &mut BTreeMap<B256, Bytes>,
    expected_hash: B256,
    code: Bytes,
    source: &'static str,
) -> eyre::Result<bool> {
    let actual_hash = keccak256(&code);
    if actual_hash != expected_hash {
        eyre::bail!(
            "{source} bytecode hash mismatch: expected={expected_hash}, actual={actual_hash}"
        );
    }

    if let Some(existing) = codes_by_hash.get(&expected_hash) {
        if existing != &code {
            eyre::bail!("conflicting {source} bytecode for hash {expected_hash}");
        }
        return Ok(false);
    }

    codes_by_hash.insert(expected_hash, code);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::iter::empty;

    use alloy_primitives::{Address, U256};
    use reth_revm::db::{states::cache_account::CacheAccount, BundleState, EmptyDB, State};
    use revm::state::{AccountInfo, Bytecode};

    use super::*;

    fn no_parent_bytecode(_: &B256) -> eyre::Result<Option<Bytes>> {
        Ok(None)
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
        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, Vec::new(), empty(), no_parent_bytecode).unwrap();

        assert!(
            codes.iter().any(|c| keccak256(c) == code_hash),
            "accessed contract code must be captured even when only on info.code"
        );
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 0);
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

        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, Vec::new(), empty(), no_parent_bytecode).unwrap();

        assert!(codes.is_empty());
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 0);
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

        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, record.codes, empty(), no_parent_bytecode).unwrap();
        assert!(
            codes.iter().any(|code| keccak256(code) == code_hash),
            "block witness codes must include every accessed account info.code"
        );
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 0);
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
            collect_accessed_codes(&state, Vec::new(), empty(), no_parent_bytecode).unwrap_err();
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
        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, vec![raw], empty(), no_parent_bytecode).unwrap();

        assert_eq!(codes.len(), 1, "duplicate code must collapse to one entry");
        assert_eq!(keccak256(&codes[0]), code_hash);
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 0);
    }

    /// A designation synthesized and cleared during one block can be absent
    /// from both reth's execution witness and every final account. The signed
    /// authorization target must still make that transient code replayable.
    #[test]
    fn collects_transient_eip7702_designations_from_authorizations() {
        let target = Address::repeat_byte(0x77);
        let designation = Bytecode::new_eip7702(target);
        let designation_hash = designation.hash_slow();
        let state = State::builder().with_database(EmptyDB::default()).build();

        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(
                &state,
                Vec::new(),
                [target, Address::ZERO, target],
                no_parent_bytecode,
            )
            .unwrap();

        assert_eq!(authorization_bytecodes_added, 1);
        assert_eq!(parent_state_bytecodes_added, 0);
        assert_eq!(codes.len(), 1);
        assert_eq!(keccak256(&codes[0]), designation_hash);
        assert_eq!(codes[0], designation.original_bytes());
    }

    /// A loaded contract can carry only its code hash because revm expects a
    /// later database lookup. Inline witness capture must perform that lookup
    /// before the parent state disappears behind a chunk boundary.
    #[test]
    fn resolves_hash_only_accessed_account_from_parent_state() {
        let address = Address::repeat_byte(0x61);
        let raw = Bytes::from_static(&[0x60, 0x01, 0x56]);
        let code_hash = keccak256(&raw);
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        state.cache.accounts.insert(
            address,
            CacheAccount::new_loaded(
                AccountInfo {
                    balance: U256::ZERO,
                    nonce: 1,
                    code_hash,
                    code: None,
                },
                Default::default(),
            ),
        );

        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, Vec::new(), empty(), |requested_hash| {
                assert_eq!(*requested_hash, code_hash);
                Ok(Some(raw.clone()))
            })
            .unwrap();

        assert_eq!(codes, vec![raw.to_vec()]);
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 1);
    }

    /// Clearing or replacing a contract removes its old code from the final
    /// account info. The bundle's original info still references that prestate
    /// code and must keep it available for guest replay.
    #[test]
    fn resolves_hash_only_bundle_prestate_code_after_account_clearing() {
        let address = Address::repeat_byte(0x62);
        let raw = Bytes::from_static(&[0x60, 0x02, 0x60, 0x00, 0x52]);
        let code_hash = keccak256(&raw);
        let original_info = AccountInfo {
            balance: U256::ZERO,
            nonce: 1,
            code_hash,
            code: None,
        };
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        state.bundle_state = BundleState::builder(0..=0)
            .state_original_account_info(address, original_info)
            .build();

        let (codes, authorization_bytecodes_added, parent_state_bytecodes_added) =
            collect_accessed_codes(&state, Vec::new(), empty(), |requested_hash| {
                assert_eq!(*requested_hash, code_hash);
                Ok(Some(raw.clone()))
            })
            .unwrap();

        assert_eq!(codes, vec![raw.to_vec()]);
        assert_eq!(authorization_bytecodes_added, 0);
        assert_eq!(parent_state_bytecodes_added, 1);
    }

    /// Missing parent code is rejected while building the payload instead of
    /// surfacing later as an opaque prover panic.
    #[test]
    fn rejects_unresolved_hash_only_account_code() {
        let address = Address::repeat_byte(0x63);
        let code_hash = B256::repeat_byte(0xa5);
        let mut state = State::builder().with_database(EmptyDB::default()).build();
        state.cache.accounts.insert(
            address,
            CacheAccount::new_loaded(
                AccountInfo {
                    balance: U256::ZERO,
                    nonce: 1,
                    code_hash,
                    code: None,
                },
                Default::default(),
            ),
        );

        let error =
            collect_accessed_codes(&state, Vec::new(), empty(), no_parent_bytecode).unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains(&address.to_string()),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&code_hash.to_string()),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("parent-state provider returned no bytes"),
            "unexpected error: {message}"
        );
    }
}
