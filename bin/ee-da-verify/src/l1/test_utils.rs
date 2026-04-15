use alpen_ee_common::parse_chunk_header;
use bitcoin::{
    absolute::LockTime,
    block::{Header, Version},
    hashes::Hash,
    key::UntweakedKeypair,
    opcodes::all::{OP_CHECKSIG, OP_RETURN},
    pow::CompactTarget,
    script::Builder,
    secp256k1::{Parity, SECP256K1},
    taproot::{ControlBlock, LeafVersion, TaprootMerkleBranch},
    transaction::Version as TxVersion,
    Amount, Block, BlockHash, OutPoint, ScriptBuf, Sequence, TapNodeHash, Transaction, TxIn,
    TxMerkleNode, TxOut, Witness, XOnlyPublicKey,
};
use proptest::{collection, prelude::*};
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;

/// Builds the exact DA linking tag script shape.
pub(crate) fn build_linking_tag_script(magic: [u8; 4], prev_wtxid: [u8; 32]) -> ScriptBuf {
    Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(magic)
        .push_slice(prev_wtxid)
        .into_script()
}

/// Builds a reveal transaction with a linking tag and envelope witness payload.
pub(crate) fn build_reveal_tx(magic: [u8; 4], prev_wtxid: [u8; 32], payload: &[u8]) -> Transaction {
    let control_block = test_control_block();
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&control_block.internal_key.serialize())
        .expect("pubkey accepted")
        .add_envelope(payload)
        .expect("envelope payload accepted")
        .build_without_min_check()
        .expect("reveal script build succeeds");

    let mut tx = Transaction {
        version: TxVersion(2),
        lock_time: LockTime::ZERO,
        input: vec![TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        }],
        output: vec![
            TxOut {
                value: Amount::from_sat(0),
                script_pubkey: build_linking_tag_script(magic, prev_wtxid),
            },
            TxOut {
                value: Amount::from_sat(1_000),
                script_pubkey: Builder::new().push_opcode(OP_CHECKSIG).into_script(),
            },
        ],
    };

    tx.input[0].witness.push([1u8; 64]);
    tx.input[0].witness.push(reveal_script);
    tx.input[0].witness.push(control_block.serialize());
    tx
}

/// Appends an additional DA linking-tag output to an existing reveal tx.
pub(crate) fn append_linking_tag_output(
    tx: &mut Transaction,
    magic: [u8; 4],
    prev_wtxid: [u8; 32],
) {
    tx.output.push(TxOut {
        value: Amount::from_sat(0),
        script_pubkey: build_linking_tag_script(magic, prev_wtxid),
    });
}

/// Builds a synthetic block with caller-provided transactions.
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

/// Builds a v0 DA chunk payload (`header ++ payload`).
///
/// This helper intentionally round-trips through [`parse_chunk_header`] so test
/// fixtures stay aligned with the production chunk-header parser contract.
pub(crate) fn build_chunk_payload(
    blob_hash: [u8; 32],
    chunk_index: u16,
    total_chunks: u16,
    body: &[u8],
) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(0);
    payload.extend_from_slice(&blob_hash);
    payload.extend_from_slice(&chunk_index.to_be_bytes());
    payload.extend_from_slice(&total_chunks.to_be_bytes());
    payload.extend_from_slice(body);

    let parsed = parse_chunk_header(&payload).expect("test chunk payload must have a valid header");
    assert_eq!(parsed.chunk_index(), chunk_index);
    assert_eq!(parsed.total_chunks(), total_chunks);
    assert_eq!(parsed.blob_hash().as_ref(), blob_hash.as_slice());

    payload
}

/// Strategy for arbitrary 4-byte tag magic.
pub(crate) fn magic_bytes_strategy() -> impl Strategy<Value = [u8; 4]> {
    any::<[u8; 4]>()
}

/// Strategy for arbitrary predecessor wtxid bytes.
pub(crate) fn prev_wtxid_strategy() -> impl Strategy<Value = [u8; 32]> {
    any::<[u8; 32]>()
}

/// Strategy for valid chunk-body bytes with caller-controlled max length.
pub(crate) fn chunk_body_strategy(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    collection::vec(any::<u8>(), 0..=max_len)
}

/// Strategy for valid chunk-header parameters (`chunk_index < total_chunks`).
pub(crate) fn valid_chunk_header_strategy() -> impl Strategy<Value = ([u8; 32], u16, u16)> {
    (any::<[u8; 32]>(), 1u16..=u16::MAX).prop_flat_map(|(blob_hash, total_chunks)| {
        (Just(blob_hash), 0u16..total_chunks, Just(total_chunks))
    })
}

fn test_control_block() -> ControlBlock {
    let keypair = UntweakedKeypair::from_seckey_slice(SECP256K1, &[7u8; 32]).expect("keypair");
    let internal_key = XOnlyPublicKey::from_keypair(&keypair).0;
    let branch: [TapNodeHash; 0] = [];

    ControlBlock {
        leaf_version: LeafVersion::TapScript,
        output_key_parity: Parity::Even,
        internal_key,
        merkle_branch: TaprootMerkleBranch::from(branch),
    }
}
