use alpen_ee_common::{DaBlob, EvmHeaderSummary};
use bitcoin::{
    absolute::LockTime,
    block::{Header, Version},
    hashes::{sha256, Hash},
    opcodes::all::OP_RETURN,
    pow::CompactTarget,
    script::Builder,
    secp256k1::{Parity, XOnlyPublicKey, SECP256K1},
    taproot::{ControlBlock, LeafVersion, TaprootMerkleBranch},
    transaction::Version as TxVersion,
    Amount, Block, BlockHash, OutPoint, ScriptBuf, Sequence, TapNodeHash, Transaction, TxIn,
    TxMerkleNode, TxOut, Txid, Witness,
};
use proptest::{collection, prelude::*};
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_l1_txfmt::MagicBytes;

use crate::ParsedEnvelope;

const BASE_TIMESTAMP: u64 = 1_700_000_000;
const BASE_GAS_LIMIT: u64 = 30_000_000;
const MIN_MULTI_CHUNK_BYTECODE_LEN: usize = 400_000;
const MAX_MULTI_CHUNK_BYTECODE_LEN: usize = 450_000;

pub(crate) fn build_commit_marker_script(magic: MagicBytes, version: u32) -> ScriptBuf {
    let mut payload = [0u8; 8];
    payload[..4].copy_from_slice(magic.as_bytes());
    payload[4..].copy_from_slice(&version.to_be_bytes());

    Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(payload)
        .into_script()
}

pub(crate) fn build_commit_tx(
    magic: MagicBytes,
    version: u32,
    reveal_slots: usize,
    add_ambiguous_p2tr_change: bool,
) -> Transaction {
    let mut output = vec![TxOut {
        value: Amount::from_sat(0),
        script_pubkey: build_commit_marker_script(magic, version),
    }];

    let reveal_key = test_xonly_public_key(3);
    for _ in 0..reveal_slots {
        output.push(TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new_p2tr(SECP256K1, reveal_key, None),
        });
    }

    output.push(TxOut {
        value: Amount::from_sat(1_000),
        script_pubkey: ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::all_zeros()),
    });

    if add_ambiguous_p2tr_change {
        output.push(TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new_p2tr(SECP256K1, reveal_key, None),
        });
    }

    Transaction {
        version: TxVersion(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output,
    }
}

pub(crate) fn build_reveal_tx(
    commit_txid: Txid,
    commit_vout: u32,
    sequencer_pubkey: XOnlyPublicKey,
    chunk: &[u8],
) -> Transaction {
    let control_block = test_control_block(sequencer_pubkey);
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&sequencer_pubkey.serialize())
        .expect("pubkey accepted")
        .add_envelope(chunk)
        .expect("envelope payload accepted")
        .build_without_min_check()
        .expect("reveal script build succeeds");

    let mut tx = Transaction {
        version: TxVersion(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint {
                txid: commit_txid,
                vout: commit_vout,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![TxOut {
            value: Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::all_zeros()),
        }],
    };

    tx.input[0].witness.push([1u8; 64]);
    tx.input[0].witness.push(reveal_script);
    tx.input[0].witness.push(control_block.serialize());
    tx
}

pub(crate) fn build_block_with_txs(txs: Vec<Transaction>) -> Block {
    Block {
        header: Header {
            version: Version::from_consensus(1),
            prev_blockhash: BlockHash::all_zeros(),
            merkle_root: TxMerkleNode::all_zeros(),
            time: 0,
            bits: CompactTarget::from_consensus(0),
            nonce: 0,
        },
        txdata: txs,
    }
}

pub(crate) fn magic_bytes_strategy() -> impl Strategy<Value = [u8; 4]> {
    any::<[u8; 4]>()
}

pub(crate) fn chunk_body_strategy(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    collection::vec(any::<u8>(), 0..=max_len)
}

pub(crate) fn make_test_blob(block_num: u64) -> DaBlob {
    DaBlob {
        update_seq_no: block_num,
        evm_header: test_evm_header(block_num),
        state_diff: Default::default(),
    }
}

pub(crate) fn make_multi_chunk_test_blob(
    block_num: u64,
    bytecode_len: usize,
    fill_byte: u8,
) -> DaBlob {
    let mut blob = make_test_blob(block_num);
    blob.state_diff
        .deployed_bytecodes
        .insert(Default::default(), vec![fill_byte; bytecode_len].into());
    blob
}

pub(crate) fn multi_chunk_bytecode_len_strategy() -> impl Strategy<Value = usize> {
    MIN_MULTI_CHUNK_BYTECODE_LEN..=MAX_MULTI_CHUNK_BYTECODE_LEN
}

pub(crate) fn build_parsed_envelope_from_chunk_bytes(chunks: Vec<Vec<u8>>) -> ParsedEnvelope {
    let txid = Txid::from_byte_array(synthetic_txid_bytes(&chunks));
    ParsedEnvelope::new(txid, chunks)
}

fn test_evm_header(block_num: u64) -> EvmHeaderSummary {
    let block_delta = block_num % 1_000_000;
    let gas_used = BASE_GAS_LIMIT / 2 + (block_delta % 1_000);

    EvmHeaderSummary {
        block_num,
        timestamp: BASE_TIMESTAMP + block_delta,
        base_fee: 1_000_000_000 + block_delta,
        gas_used,
        gas_limit: BASE_GAS_LIMIT + (block_delta % 1_000),
    }
}

fn test_xonly_public_key(seed: u8) -> XOnlyPublicKey {
    use bitcoin::secp256k1::{Keypair, Secp256k1, SecretKey};

    let secp = Secp256k1::new();
    let secret_key = SecretKey::from_slice(&[seed; 32]).expect("valid secret key");
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    XOnlyPublicKey::from_keypair(&keypair).0
}

fn test_control_block(internal_key: XOnlyPublicKey) -> ControlBlock {
    let branch: [TapNodeHash; 0] = [];

    ControlBlock {
        leaf_version: LeafVersion::TapScript,
        output_key_parity: Parity::Even,
        internal_key,
        merkle_branch: TaprootMerkleBranch::from(branch),
    }
}

fn synthetic_txid_bytes(chunks: &[Vec<u8>]) -> [u8; 32] {
    let mut seed = Vec::new();
    for chunk in chunks {
        seed.extend_from_slice(&chunk.len().to_le_bytes());
        seed.extend_from_slice(chunk);
    }
    sha256::Hash::hash(&seed).to_byte_array()
}
