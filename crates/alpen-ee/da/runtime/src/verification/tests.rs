use std::collections::BTreeMap;

use alloy_primitives::{keccak256, Address, Bytes, U256};
use alpen_ee_da_types::{
    ArchivedDaWitness, BitcoinMerkleProof, BytecodePreimage, DaBlob, DaBlockWitness, DaParseError,
    DaTxWitness, DaWitness, DedupWitness, EvmHeaderSummary, L1DaBlockInclusion, DA_BLOB_VERSION,
};
use alpen_reth_statediff::{
    apply_batch_state_diff_to_ethereum_state, AccountChange, AccountDiff, BatchStateDiff,
};
use bitcoin::{
    absolute::LockTime,
    consensus::serialize,
    hashes::Hash as _,
    key::UntweakedKeypair,
    opcodes::all::OP_RETURN,
    script,
    secp256k1::SECP256K1,
    taproot::{ControlBlock, LeafVersion, TaprootBuilder},
    transaction::Version,
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey,
};
use rkyv::rancor::Error as RkyvError;
use rsp_mpt::EthereumState;
use sha2::{Digest, Sha256};
use strata_acct_types::{l1_block_record_leaf_hash, Hash};
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{ArchivedEePrivateInput, ChunkInput, EePrivateInput};
use strata_ee_chain_types::{ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs};
use strata_evm_ee::EvmPartialState;
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefs, ProofState, Seqno, UpdateOutputs, UpdateProofPubParams,
};

use super::{verify_da_witness, DaVerificationError};

const MAGIC: [u8; 4] = *b"ALPN";

fn hash_pair(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
    let mut pair = [0u8; 64];
    pair[..32].copy_from_slice(&left);
    pair[32..].copy_from_slice(&right);
    let first = Sha256::digest(pair);
    Sha256::digest(first).into()
}

fn commit_tx() -> Transaction {
    let mut payload = [0u8; 8];
    payload[..4].copy_from_slice(&MAGIC);
    payload[4..].copy_from_slice(&DA_BLOB_VERSION.to_be_bytes());

    let p2tr_script = {
        let mut bytes = vec![0x51, 0x20];
        bytes.extend_from_slice(&[0u8; 32]);
        ScriptBuf::from_bytes(bytes)
    };

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::new(),
        output: vec![
            TxOut {
                value: Amount::ZERO,
                script_pubkey: script::Builder::new()
                    .push_opcode(OP_RETURN)
                    .push_slice(payload)
                    .into_script(),
            },
            TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: p2tr_script,
            },
        ],
    }
}

fn reveal_tx(commit_txid: Txid, chunk: &[u8]) -> Transaction {
    reveal_tx_spending_vout(commit_txid, 1, chunk)
}

fn reveal_tx_spending_vout(commit_txid: Txid, vout: u32, chunk: &[u8]) -> Transaction {
    let secret_bytes = [0x42u8; 32];
    let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &secret_bytes).unwrap();
    let pubkey = XOnlyPublicKey::from_keypair(&key_pair).0;
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&pubkey.serialize())
        .unwrap()
        .add_envelopes(&[chunk.to_vec()])
        .unwrap()
        .build_without_min_check()
        .unwrap();

    let spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .unwrap()
        .finalize(SECP256K1, pubkey)
        .unwrap();
    let control_block: ControlBlock = spend_info
        .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
        .unwrap();

    let mut witness = Witness::new();
    witness.push([0u8; 64]);
    witness.push(reveal_script.as_bytes());
    witness.push(control_block.serialize());

    Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: commit_txid,
                vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness,
        }],
        output: vec![TxOut {
            value: Amount::from_sat(500),
            script_pubkey: ScriptBuf::new(),
        }],
    }
}

fn archive_inputs(ee_input: &EePrivateInput, da_witness: &DaWitness) -> (Vec<u8>, Vec<u8>) {
    let ee_bytes = rkyv::to_bytes::<RkyvError>(ee_input).unwrap().to_vec();
    let da_bytes = rkyv::to_bytes::<RkyvError>(da_witness).unwrap().to_vec();
    (ee_bytes, da_bytes)
}

fn run_verify_da_witness(
    ee_input: &EePrivateInput,
    da_witness: &DaWitness,
    pub_params: &UpdateProofPubParams,
    expected_pre_root: [u8; 32],
) -> Result<(), DaVerificationError> {
    let (ee_bytes, da_bytes) = archive_inputs(ee_input, da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    verify_da_witness(archived_ee, archived_da, pub_params, expected_pre_root)
}

fn rebuild_pub_params(
    seq_no: u64,
    height: u32,
    block_hash: [u8; 32],
    wtxids_root: [u8; 32],
) -> UpdateProofPubParams {
    let ledger_ref_hash = l1_block_record_leaf_hash(&block_hash, &wtxids_root);
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(height as u64, ledger_ref_hash)]);

    UpdateProofPubParams::new(
        Seqno::new(seq_no),
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        ledger_refs,
        UpdateOutputs::new_empty(),
        Vec::new(),
    )
}

fn rebuild_ee_input(
    raw_partial_pre_state: &[u8],
    tip_state_root: [u8; 32],
    header: EvmHeaderSummary,
) -> EePrivateInput {
    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from(tip_state_root),
        ExecHeaderSummary::from_vec(encode_to_vec(&header).unwrap()).unwrap(),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    EePrivateInput::new(
        Vec::new(),
        raw_partial_pre_state.to_vec(),
        vec![ChunkInput::new(transition, Vec::new())],
    )
}

fn valid_fixture() -> (EePrivateInput, DaWitness, UpdateProofPubParams, [u8; 32]) {
    let header = EvmHeaderSummary {
        block_num: 10,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    let pre_state = EvmPartialState::new(
        EthereumState {
            state_trie: Default::default(),
            storage_tries: Default::default(),
        },
        BTreeMap::new(),
        Vec::new(),
    );
    let pre_root = pre_state.ethereum_state().state_root().0;
    let raw_pre_state = encode_to_vec(&pre_state).unwrap();

    let blob = DaBlob {
        update_seq_no: 7,
        evm_header: header,
        state_diff: BatchStateDiff::new(),
    };
    let chunks = [encode_to_vec(&blob).unwrap()];
    assert_eq!(chunks.len(), 1);

    let commit = commit_tx();
    let reveal = reveal_tx(commit.compute_txid(), &chunks[0]);
    let commit_wtxid = commit.compute_wtxid().to_byte_array();
    let reveal_wtxid = reveal.compute_wtxid().to_byte_array();
    let wtxids_root = hash_pair(commit_wtxid, reveal_wtxid);
    let block_hash = [0x44; 32];
    let height = 42;

    let ee_input = rebuild_ee_input(&raw_pre_state, pre_root, header);

    let da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(height, block_hash, wtxids_root),
            vec![
                DaTxWitness::new(
                    serialize(&commit),
                    BitcoinMerkleProof::new(vec![reveal_wtxid], 0),
                ),
                DaTxWitness::new(
                    serialize(&reveal),
                    BitcoinMerkleProof::new(vec![commit_wtxid], 1),
                ),
            ],
        )],
        DedupWitness::empty(),
    );

    let pub_params = rebuild_pub_params(blob.update_seq_no, height, block_hash, wtxids_root);

    (ee_input, da_witness, pub_params, pre_root)
}

#[test]
fn verify_da_witness_accepts_deduped_bytecode_from_private_witness() {
    let header = EvmHeaderSummary {
        block_num: 10,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    let mut pre_state = EvmPartialState::new(
        EthereumState {
            state_trie: Default::default(),
            storage_tries: Default::default(),
        },
        BTreeMap::new(),
        Vec::new(),
    );
    let pre_root = pre_state.ethereum_state().state_root().0;
    let raw_pre_state = encode_to_vec(&pre_state).unwrap();

    let bytecode = Bytes::from_static(&[0x60, 0x80, 0x60, 0x40, 0x52]);
    let code_hash = keccak256(bytecode.as_ref());
    let mut state_diff = BatchStateDiff::new();
    state_diff.accounts.insert(
        Address::from([0x11; 20]),
        AccountChange::Created(AccountDiff::new_created(U256::ZERO, 1, code_hash)),
    );
    apply_batch_state_diff_to_ethereum_state(pre_state.ethereum_state_mut(), &state_diff).unwrap();
    let post_root = pre_state.ethereum_state().state_root().0;

    let blob = DaBlob {
        update_seq_no: 7,
        evm_header: header,
        state_diff,
    };
    let chunks = [encode_to_vec(&blob).unwrap()];
    let commit = commit_tx();
    let reveal = reveal_tx(commit.compute_txid(), &chunks[0]);
    let commit_wtxid = commit.compute_wtxid().to_byte_array();
    let reveal_wtxid = reveal.compute_wtxid().to_byte_array();
    let wtxids_root = hash_pair(commit_wtxid, reveal_wtxid);
    let block_hash = [0x44; 32];
    let height = 42;
    let ledger_ref_hash = l1_block_record_leaf_hash(&block_hash, &wtxids_root);
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(height as u64, ledger_ref_hash)]);

    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from(post_root),
        ExecHeaderSummary::from_vec(encode_to_vec(&header).unwrap()).unwrap(),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let ee_input = EePrivateInput::new(
        Vec::new(),
        raw_pre_state,
        vec![ChunkInput::new(transition, Vec::new())],
    );
    let da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(height, block_hash, wtxids_root),
            vec![
                DaTxWitness::new(
                    serialize(&commit),
                    BitcoinMerkleProof::new(vec![reveal_wtxid], 0),
                ),
                DaTxWitness::new(
                    serialize(&reveal),
                    BitcoinMerkleProof::new(vec![commit_wtxid], 1),
                ),
            ],
        )],
        DedupWitness::new(vec![BytecodePreimage::new(bytecode.to_vec())]),
    );
    let pub_params = UpdateProofPubParams::new(
        Seqno::new(blob.update_seq_no),
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        ledger_refs,
        UpdateOutputs::new_empty(),
        Vec::new(),
    );
    let (ee_bytes, da_bytes) = archive_inputs(&ee_input, &da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    verify_da_witness(archived_ee, archived_da, &pub_params, pre_root)
        .expect("private witness bytecode should satisfy deduped code hash");
}

#[test]
fn verify_da_blob_metadata_rejects_missing_deployed_bytecode() {
    let header = EvmHeaderSummary {
        block_num: 10,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    // An account references a code hash that is neither published in the blob nor
    // supplied as a preimage, so verification must reject it.
    let code_hash = keccak256([0x60, 0x80]);
    let mut state_diff = BatchStateDiff::new();
    state_diff.accounts.insert(
        Address::from([0x11; 20]),
        AccountChange::Created(AccountDiff::new_created(U256::ZERO, 1, code_hash)),
    );
    let blob = DaBlob {
        update_seq_no: 7,
        evm_header: header,
        state_diff,
    };
    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from([3; 32]),
        ExecHeaderSummary::from_vec(encode_to_vec(&header).unwrap()).unwrap(),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let pub_params = UpdateProofPubParams::new(
        Seqno::new(blob.update_seq_no),
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        Vec::new(),
    );
    let witness = DaWitness::new(Vec::new(), DedupWitness::empty());
    let da_bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    let err = super::verify_da_blob_metadata(
        &blob,
        &transition,
        &pub_params,
        archived_da.dedup_da_witness(),
    )
    .expect_err("account code hash with no published or witnessed bytecode must fail");

    assert!(matches!(
        err,
        DaVerificationError::MissingDeployedBytecode(_)
    ));
}

#[test]
fn verify_da_witness_rejects_empty_witness_for_non_empty_batch() {
    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from([3; 32]),
        ExecHeaderSummary::new_empty(),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let ee_input = EePrivateInput::new(
        Vec::new(),
        Vec::new(),
        vec![ChunkInput::new(transition, Vec::new())],
    );
    let da_witness = DaWitness::empty();
    let pub_params = UpdateProofPubParams::new(
        Seqno::new(1),
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        Vec::new(),
    );
    let (ee_bytes, da_bytes) = archive_inputs(&ee_input, &da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    let err = verify_da_witness(archived_ee, archived_da, &pub_params, [0; 32])
        .expect_err("non-empty batch must require DA witness");

    assert!(matches!(err, DaVerificationError::MissingDaWitness));
}

#[test]
fn verify_da_witness_accepts_valid_commit_reveal_round_trip() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let (ee_bytes, da_bytes) = archive_inputs(&ee_input, &da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    verify_da_witness(archived_ee, archived_da, &pub_params, expected_pre_root)
        .expect("valid DA witness must verify");
}

#[test]
fn verify_da_witness_rejects_update_seq_no_mismatch() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    let bad_pub_params = rebuild_pub_params(
        *pub_params.seq_no().inner() + 1,
        block.inclusion().l1_block_height(),
        *block.inclusion().l1_block_hash(),
        *block.inclusion().wtxids_root(),
    );

    let err = run_verify_da_witness(&ee_input, &da_witness, &bad_pub_params, expected_pre_root)
        .expect_err("DA blob seq_no must match public proof params");

    assert!(matches!(
        err,
        DaVerificationError::UpdateSeqNoMismatch {
            expected: 8,
            actual: 7
        }
    ));
}

#[test]
fn verify_da_witness_rejects_evm_header_mismatch() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let wrong_header = EvmHeaderSummary {
        block_num: 11,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    let bad_ee_input = rebuild_ee_input(
        ee_input.raw_partial_pre_state(),
        expected_pre_root,
        wrong_header,
    );

    let err = run_verify_da_witness(&bad_ee_input, &da_witness, &pub_params, expected_pre_root)
        .expect_err("DA blob header must match the last chunk public header summary");

    assert!(matches!(err, DaVerificationError::EvmHeaderMismatch { .. }));
}

#[test]
fn verify_da_witness_rejects_state_root_mismatch() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let mut wrong_tip_state_root = expected_pre_root;
    wrong_tip_state_root[0] ^= 1;
    let header = EvmHeaderSummary {
        block_num: 10,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    let bad_ee_input = rebuild_ee_input(
        ee_input.raw_partial_pre_state(),
        wrong_tip_state_root,
        header,
    );

    let err = run_verify_da_witness(&bad_ee_input, &da_witness, &pub_params, expected_pre_root)
        .expect_err("post-apply state root must match the last chunk tip_state_root");

    assert!(matches!(
        err,
        DaVerificationError::PostApplyStateRootMismatch { .. }
    ));
}

#[test]
fn verify_da_witness_rejects_missing_reveal() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    let commit_tx = &block.txs()[0];
    let da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(
                block.inclusion().l1_block_height(),
                *block.inclusion().l1_block_hash(),
                *block.inclusion().wtxids_root(),
            ),
            vec![DaTxWitness::new(
                commit_tx.raw_tx().to_vec(),
                commit_tx.wtxid_inclusion_proof().clone(),
            )],
        )],
        DedupWitness::empty(),
    );

    let err = run_verify_da_witness(&ee_input, &da_witness, &pub_params, expected_pre_root)
        .expect_err("commit output 1 must have a matching reveal");

    assert!(matches!(
        err,
        DaVerificationError::Parse(DaParseError::MissingReveal(1))
    ));
}

#[test]
fn verify_da_witness_rejects_duplicate_reveal() {
    let (ee_input, da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    let commit_tx = &block.txs()[0];
    let reveal_tx = &block.txs()[1];
    let duplicate_reveal = DaTxWitness::new(
        reveal_tx.raw_tx().to_vec(),
        reveal_tx.wtxid_inclusion_proof().clone(),
    );
    let da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(
                block.inclusion().l1_block_height(),
                *block.inclusion().l1_block_hash(),
                *block.inclusion().wtxids_root(),
            ),
            vec![
                DaTxWitness::new(
                    commit_tx.raw_tx().to_vec(),
                    commit_tx.wtxid_inclusion_proof().clone(),
                ),
                DaTxWitness::new(
                    reveal_tx.raw_tx().to_vec(),
                    reveal_tx.wtxid_inclusion_proof().clone(),
                ),
                duplicate_reveal,
            ],
        )],
        DedupWitness::empty(),
    );

    let err = run_verify_da_witness(&ee_input, &da_witness, &pub_params, expected_pre_root)
        .expect_err("two reveals for the same commit output must fail");

    assert!(matches!(
        err,
        DaVerificationError::Parse(DaParseError::DuplicateReveal(1))
    ));
}

#[test]
fn verify_da_witness_rejects_reveal_spending_marker_vout() {
    let (ee_input, _, pub_params, expected_pre_root) = valid_fixture();
    let commit = commit_tx();
    let reveal = reveal_tx_spending_vout(commit.compute_txid(), 0, &[0xaa]);
    let commit_wtxid = commit.compute_wtxid().to_byte_array();
    let reveal_wtxid = reveal.compute_wtxid().to_byte_array();
    let wtxids_root = hash_pair(commit_wtxid, reveal_wtxid);
    let block_hash = [0x45; 32];
    let height = 43;
    let da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(height, block_hash, wtxids_root),
            vec![
                DaTxWitness::new(
                    serialize(&commit),
                    BitcoinMerkleProof::new(vec![reveal_wtxid], 0),
                ),
                DaTxWitness::new(
                    serialize(&reveal),
                    BitcoinMerkleProof::new(vec![commit_wtxid], 1),
                ),
            ],
        )],
        DedupWitness::empty(),
    );
    let pub_params =
        rebuild_pub_params(*pub_params.seq_no().inner(), height, block_hash, wtxids_root);

    let err = run_verify_da_witness(&ee_input, &da_witness, &pub_params, expected_pre_root)
        .expect_err("a reveal cannot spend the commit OP_RETURN marker output");

    assert!(matches!(
        err,
        DaVerificationError::Parse(DaParseError::RevealSpendsMarker)
    ));
}

#[test]
fn verify_da_witness_rejects_unclaimed_l1_ref() {
    let (ee_input, mut da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(
                block.inclusion().l1_block_height() + 1,
                *block.inclusion().l1_block_hash(),
                *block.inclusion().wtxids_root(),
            ),
            block.txs().to_vec(),
        )],
        DedupWitness::empty(),
    );
    let (ee_bytes, da_bytes) = archive_inputs(&ee_input, &da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    let err = verify_da_witness(archived_ee, archived_da, &pub_params, expected_pre_root)
        .expect_err("unclaimed L1 ref must fail");

    assert!(matches!(
        err,
        DaVerificationError::L1DaBlockRefNotInLedgerRefs { .. }
    ));
}

#[test]
fn verify_da_witness_rejects_bad_wtxid_root() {
    let (ee_input, mut da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    let commit_tx = &block.txs()[0];
    let reveal_tx = &block.txs()[1];
    da_witness = DaWitness::new(
        vec![DaBlockWitness::new(
            L1DaBlockInclusion::new(
                block.inclusion().l1_block_height(),
                *block.inclusion().l1_block_hash(),
                *block.inclusion().wtxids_root(),
            ),
            vec![
                DaTxWitness::new(
                    commit_tx.raw_tx().to_vec(),
                    commit_tx.wtxid_inclusion_proof().clone(),
                ),
                DaTxWitness::new(
                    reveal_tx.raw_tx().to_vec(),
                    BitcoinMerkleProof::new(vec![[0x99; 32]], 1),
                ),
            ],
        )],
        DedupWitness::empty(),
    );
    let (ee_bytes, da_bytes) = archive_inputs(&ee_input, &da_witness);
    let archived_ee = rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_bytes).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    let err = verify_da_witness(archived_ee, archived_da, &pub_params, expected_pre_root)
        .expect_err("bad wtxid root must fail");

    assert!(matches!(
        err,
        DaVerificationError::WtxidsRootMismatch { .. }
    ));
}
