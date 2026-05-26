use std::collections::BTreeMap;

use alloy_primitives::{Address, Bytes, U256, keccak256};
use alpen_reth_statediff::{
    AccountChange, AccountDiff, BatchStateDiff, apply_batch_state_diff_to_ethereum_state,
};
use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness, XOnlyPublicKey,
    absolute::LockTime,
    consensus::serialize,
    hashes::Hash as _,
    key::UntweakedKeypair,
    opcodes::all::OP_RETURN,
    script,
    secp256k1::SECP256K1,
    taproot::{ControlBlock, LeafVersion, TaprootBuilder},
    transaction::Version,
};
use rkyv::rancor::Error as RkyvError;
use rsp_mpt::EthereumState;
use sha2::{Digest, Sha256};
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_runtime::{
    ArchivedDaWitness, ArchivedEePrivateInput, BitcoinMerkleProof, ChunkInput, DaBlockWitness,
    DaBytecodeWitness, DaTxWitness, DaWitness, EePrivateInput, L1DaBlockInclusion,
};
use strata_ee_chain_types::{ChunkTransition, ExecHeaderSummary, ExecInputs, ExecOutputs};
use strata_evm_ee::EvmPartialState;
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefs, ProofState, UpdateOutputs, UpdateProofPubParams,
};

use super::{
    DaBlob, DaVerificationError, EvmHeaderSummary, constants::DA_BLOB_VERSION,
    inclusion::l1_block_ref_commitment, verify_da_witness,
};

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
                vout: 1,
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
    let ledger_ref_hash = l1_block_ref_commitment(&block_hash, &wtxids_root);
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(height as u64, ledger_ref_hash)]);

    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from(pre_root),
        ExecHeaderSummary::new(encode_to_vec(&header).unwrap()),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let ee_input = EePrivateInput::new(
        Vec::new(),
        raw_pre_state,
        vec![ChunkInput::new(transition, Vec::new())],
    );

    let da_witness = DaWitness::new(vec![DaBlockWitness::new(
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
    )]);

    let pub_params = UpdateProofPubParams::new(
        blob.update_seq_no,
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        ledger_refs,
        UpdateOutputs::new_empty(),
        Vec::new(),
    );

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
    let ledger_ref_hash = l1_block_ref_commitment(&block_hash, &wtxids_root);
    let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(height as u64, ledger_ref_hash)]);

    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from(post_root),
        ExecHeaderSummary::new(encode_to_vec(&header).unwrap()),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let ee_input = EePrivateInput::new(
        Vec::new(),
        raw_pre_state,
        vec![ChunkInput::new(transition, Vec::new())],
    );
    let da_witness = DaWitness::new_with_known_bytecodes(
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
        vec![DaBytecodeWitness::new(code_hash.0, bytecode.to_vec())],
    );
    let pub_params = UpdateProofPubParams::new(
        blob.update_seq_no,
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
fn verify_da_blob_metadata_rejects_known_bytecode_hash_mismatch() {
    let header = EvmHeaderSummary {
        block_num: 10,
        timestamp: 1_700_000_000,
        base_fee: 100,
        gas_used: 21_000,
        gas_limit: 30_000_000,
    };
    let blob = DaBlob {
        update_seq_no: 7,
        evm_header: header,
        state_diff: BatchStateDiff::new(),
    };
    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from([3; 32]),
        ExecHeaderSummary::new(encode_to_vec(&header).unwrap()),
        ExecInputs::new_empty(),
        ExecOutputs::new_empty(),
    );
    let pub_params = UpdateProofPubParams::new(
        blob.update_seq_no,
        ProofState::new(Hash::zero(), 0),
        ProofState::new(Hash::zero(), 0),
        Vec::new(),
        LedgerRefs::new_empty(),
        UpdateOutputs::new_empty(),
        Vec::new(),
    );
    let expected_hash = keccak256([0x60, 0x80]);
    let witness = DaWitness::new_with_known_bytecodes(
        Vec::new(),
        vec![DaBytecodeWitness::new(expected_hash.0, vec![0x61, 0x80])],
    );
    let da_bytes = rkyv::to_bytes::<RkyvError>(&witness).unwrap();
    let archived_da = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_bytes).unwrap();

    let err = super::blob::verify_da_blob_metadata(
        &blob,
        &transition,
        &pub_params,
        archived_da.known_bytecodes(),
    )
    .expect_err("known bytecode witness must hash to its claimed code hash");

    assert!(matches!(
        err,
        DaVerificationError::KnownBytecodeHashMismatch { .. }
    ));
}

#[test]
fn verify_da_witness_rejects_empty_witness_for_non_empty_batch() {
    let transition = ChunkTransition::new(
        Hash::from([1; 32]),
        Hash::from([2; 32]),
        Hash::from([3; 32]),
        ExecHeaderSummary::new(Vec::new()),
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
        1,
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

    let blob = verify_da_witness(archived_ee, archived_da, &pub_params, expected_pre_root)
        .expect("valid DA witness must verify")
        .expect("non-empty witness must produce DA blob");

    assert_eq!(blob.update_seq_no, pub_params.seq_no());
}

#[test]
fn verify_da_witness_rejects_unclaimed_l1_ref() {
    let (ee_input, mut da_witness, pub_params, expected_pre_root) = valid_fixture();
    let block = da_witness.blocks().first().unwrap();
    da_witness = DaWitness::new(vec![DaBlockWitness::new(
        L1DaBlockInclusion::new(
            block.inclusion().l1_block_height() + 1,
            *block.inclusion().l1_block_hash(),
            *block.inclusion().wtxids_root(),
        ),
        block.txs().to_vec(),
    )]);
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
    da_witness = DaWitness::new(vec![DaBlockWitness::new(
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
    )]);
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
