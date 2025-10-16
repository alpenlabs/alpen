//! Btc related test utilities for the Alpen codebase.

use std::str::FromStr;

use bitcoin::{
    absolute::LockTime,
    consensus::{self, deserialize},
    hashes::Hash,
    key::TapTweak,
    opcodes::all::OP_RETURN,
    script::{self, PushBytesBuf},
    secp256k1::{Keypair, Message, Secp256k1},
    sighash::{Prevouts, SighashCache},
    Address, Amount, Block, OutPoint, ScriptBuf, Sequence, TapNodeHash, TapSighashType,
    Transaction, TxIn, TxOut, Witness,
};
use strata_asm_types::L1HeaderRecord;
use strata_bridge_types::{
    DepositEntry, DepositState, DispatchCommand, DispatchedState, WithdrawOutput,
};
use strata_params::DepositTxParams;
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    buf::Buf32,
    l1::{BitcoinAddress, BitcoinAmount, BitcoinOutPoint},
};
use strata_test_utils::ArbitraryGenerator;

pub mod segment;

pub fn get_test_bitcoin_txs() -> Vec<Transaction> {
    let t1 = "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000";
    let t2 = "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0000000000fdffffff02969e0700000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff60f59000000000001600148d0499ec043b1921a608d24690b061196e57c927040047304402203875f7b610f8783d5f5c163118eeec1a23473dd33b53c8ea584c7d28a82b209b022034b6814344b79826a348e23cc19ff06ed2df23850b889557552e376bf9e32c560147304402200f647dad3c137ff98d7da7a302345c82a57116a3d0e6a3719293bbb421cb0abe02201c04a1e808f5bab3595f77985af91aeaf61e9e042c9ac97d696e0f4b020cb54b0169522102dba8352965522ff44538dde37d793b3b4ece54e07759ade5f648aa396165d2962103c0683712773b725e7fe4809cbc90c9e0b890c45e5e24a852a4c472d1b6e9fd482103bf56f172d0631a7f8ae3ef648ad43a816ad01de4137ba89ebc33a2da8c48531553ae00000000";
    let t3 = "02000000000101f4f2e8830d2948b5e980e739e61b23f048d03d4af81588bf5da4618406c495aa0200000000ffffffff0380969800000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a15006e1a916a60b93a545f2370f2a36d2f807fb3d675588b693a000000001600149fafc79c72d1c4d917a360f32bdc68755402ef670247304402203c813ad8918366ce872642368b57b78e78e03b1a1eafe16ec8f3c9268b4fc050022018affe880963f18bfc0338f1e54c970185aa90f8c36a52ac935fe76cb885d726012102fa9b81d082a98a46d0857d62e6c9afe9e1bf40f9f0cbf361b96241c9d6fb064b00000000";
    let t4 = "02000000000101d8acf0a647b7d5d1d0ee83360158d5bf01146d3762c442defd7985476b02aa6b0100000000fdffffff030065cd1d000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f3134945466aec19dd00000000022512040718748dbca6dea8ac6b6f0b177014f0826478f1613c2b489e738db7ecdf3610247304402207cfc5cd87ec83687c9ac2bd921e96b8a58710f15d77bc7624da4fb29fe589dab0220437b74ed8e8f9d3084269edfb8641bf27246b0e5476667918beba73025c7a2c501210249a34cfbb6163b1b6ca2fff63fd1f8a802fb1999fa7930b2febe5a711f713dd900000000";
    let t5 = "0200000000010176f29f18c5fc677ad6dd6c9309f6b9112f83cb95889af21da4be7fbfe22d1d220000000000fdffffff0300e1f505000000002200203946555814a18ccc94ef4991fb6af45278425e6a0a2cfc2bf4cf9c47515c56ff0000000000000000176a1500e0e78c8201d91f362c2ad3bb6f8e6f31349454663b1010240100000022512012d77c9ae5fdca5a3ab0b17a29b683fd2690f5ad56f6057a000ec42081ac89dc0247304402205de15fbfb413505a3563608dad6a73eb271b4006a4156eeb62d1eacca5efa10b02201eb71b975304f3cbdc664c6dd1c07b93ac826603309b3258cb92cfd201bb8792012102f55f96fd587a706a7b5e7312c4e9d755a65b3dad9945d65598bca34c9e961db400000000";
    [t1, t2, t3, t4, t5]
        .iter()
        .map(|x| deserialize(&hex::decode(x).unwrap()).unwrap())
        .collect()
}

pub fn gen_l1_chain(len: usize) -> Vec<L1HeaderRecord> {
    // FIXME this is bad, the blocks generated are nonsensical
    let mut blocks = vec![];
    for _ in 0..len {
        let block: L1HeaderRecord = ArbitraryGenerator::new().generate();
        blocks.push(block);
    }
    blocks
}

pub fn get_btc_mainnet_block() -> Block {
    let raw_block = include_bytes!(
        "../../data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw"
    );
    let block: Block = deserialize(&raw_block[..]).unwrap();
    block
}

/// Creates a signed test Taproot deposit transaction.
///
/// Generates a dummy input referencing a random previous output, and constructs a
/// transaction with two outputs:
/// - A payment to `out_script_pubkey` with the specified amount.
/// - An OP_RETURN output using `opreturn_script` with zero value.
///
/// The input is signed using Taproot key spend with `SIGHASH_DEFAULT`, and the address
/// is derived from the provided `keypair` and `tapnode_hash`.
///
/// # Arguments
/// - `amt`: Amount to deposit.
/// - `out_script_pubkey`: Script to spend to.
/// - `opreturn_script`: Script for the OP_RETURN output. This contains the metadata for the
///   deposit.
/// - `keypair`: Keypair(untweaked) used to sign the transaction.
/// - `tapnode_hash`: Optional Taproot node hash for script path commitment.
///
/// # Returns
/// A signed [`Transaction`] ready for testing or simulation.
pub fn create_test_deposit_tx(
    amt: Amount,
    out_script_pubkey: &ScriptBuf,
    opreturn_script: &ScriptBuf,
    keypair: &Keypair,
    tapnode_hash: &[u8; 32],
) -> Transaction {
    let mut previous_output: BitcoinOutPoint = ArbitraryGenerator::new().generate();
    previous_output.0.vout = 0;

    let secp = Secp256k1::new();
    let (xpk, _) = keypair.x_only_public_key();
    let tapscript_root = TapNodeHash::from_byte_array(*tapnode_hash); // since there is only one
                                                                      // script node
    let sbuf = ScriptBuf::new_p2tr(
        &secp,
        xpk,
        Some(TapNodeHash::from_byte_array(*tapnode_hash)),
    );

    let prev_txout = TxOut {
        value: amt,
        script_pubkey: sbuf,
    };

    let inputs = vec![TxIn {
        previous_output: *previous_output.outpoint(),
        script_sig: Default::default(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }];

    // Construct the outputs
    let outputs = vec![
        TxOut {
            value: amt,
            script_pubkey: out_script_pubkey.clone(),
        },
        TxOut {
            value: Amount::ZERO, // Amount is zero for OP_RETURN
            script_pubkey: opreturn_script.clone(),
        },
    ];

    // Create the transaction
    let mut tx = Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    };

    let prevtxout = [prev_txout];
    let prevouts = Prevouts::All(&prevtxout);
    let sighash = SighashCache::new(&mut tx)
        .taproot_key_spend_signature_hash(0, &prevouts, TapSighashType::Default)
        .unwrap();

    let msg = Message::from_digest(*sighash.as_ref());

    let tweaked_pair = keypair.tap_tweak(&secp, Some(tapscript_root));

    // Sign the sighash
    let sig = secp.sign_schnorr(&msg, &tweaked_pair.to_keypair());

    tx.input[0].witness.push(sig.as_ref());

    tx
}

pub fn build_no_op_deposit_request_script(
    magic: Vec<u8>,
    dummy_block: Vec<u8>,
    dest_addr: Vec<u8>,
) -> ScriptBuf {
    let builder = script::Builder::new()
        .push_slice(PushBytesBuf::try_from(magic).unwrap())
        .push_slice(PushBytesBuf::try_from(dummy_block).unwrap())
        .push_slice(PushBytesBuf::try_from(dest_addr).unwrap());

    builder.into_script()
}

pub fn build_test_deposit_request_script(
    magic: Vec<u8>,
    dummy_block: Vec<u8>,
    dest_addr: Vec<u8>,
) -> ScriptBuf {
    let mut data = magic;
    data.extend(dummy_block);
    data.extend(dest_addr);
    let builder = script::Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(PushBytesBuf::try_from(data).unwrap());

    builder.into_script()
}

pub fn build_test_deposit_script(
    dep_config: &DepositTxParams,
    idx: u32,
    dest_addr: Vec<u8>,
    tapnode_hash: &[u8; 32],
) -> ScriptBuf {
    let mut data = dep_config.magic_bytes.clone().to_vec();
    data.extend(&idx.to_be_bytes()[..]);
    data.extend(dest_addr);
    data.extend(tapnode_hash);
    data.extend(&dep_config.deposit_amount.to_sat().to_be_bytes());

    let builder = script::Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(PushBytesBuf::try_from(data).unwrap());

    builder.into_script()
}

pub fn test_taproot_addr() -> BitcoinAddress {
    let addr =
        Address::from_str("bcrt1pnmrmugapastum8ztvgwcn8hvq2avmcwh2j4ssru7rtyygkpqq98q4wyd6s")
            .unwrap()
            .require_network(bitcoin::Network::Regtest)
            .unwrap();

    BitcoinAddress::parse(&addr.to_string(), bitcoin::Network::Regtest).unwrap()
}

/// Creates a test deposit transaction with a specified amount and destination address.
pub fn generate_withdrawal_fulfillment_data(
    deposit_amt: BitcoinAmount,
) -> (Vec<Descriptor>, Vec<[u8; 32]>, Vec<DepositEntry>) {
    let mut gen = ArbitraryGenerator::new();
    let mut addresses = Vec::new();
    let mut txids = Vec::<[u8; 32]>::new();
    for _ in 0..10 {
        addresses.push(Descriptor::new_p2wpkh(&gen.generate()));
        txids.push(gen.generate());
    }

    let create_outputref = |txid: &[u8; 32], vout: u32| {
        OutPoint::new(consensus::deserialize(txid).unwrap(), vout).into()
    };

    let create_dispatched_deposit_entry =
        |operator_idx: u32,
         deposit_idx: u32,
         addr: Descriptor,
         deadline: u64,
         deposit_txid: &[u8; 32],
         withdrawal_request_txid: Option<Buf32>| {
            DepositEntry::new(
                deposit_idx,
                create_outputref(deposit_txid, 0),
                vec![0, 1, 2],
                deposit_amt,
                withdrawal_request_txid,
            )
            .with_state(DepositState::Dispatched(DispatchedState::new(
                DispatchCommand::new(vec![WithdrawOutput::new(
                    addr,
                    Amount::from_btc(10.0).unwrap().into(),
                )]),
                operator_idx,
                deadline,
            )))
        };

    let deposits = vec![
        // deposits with withdrawal assignments
        create_dispatched_deposit_entry(1, 2, addresses[0].clone(), 100, &txids[0], gen.generate()),
        create_dispatched_deposit_entry(2, 3, addresses[1].clone(), 100, &txids[1], gen.generate()),
        create_dispatched_deposit_entry(0, 4, addresses[2].clone(), 100, &txids[2], gen.generate()),
        // deposits without withdrawal assignments
        DepositEntry::new(
            5,
            create_outputref(&txids[3], 0),
            vec![0, 1, 2],
            deposit_amt,
            None,
        )
        .with_state(DepositState::Accepted),
        DepositEntry::new(
            6,
            create_outputref(&txids[4], 0),
            vec![0, 1, 2],
            deposit_amt,
            None,
        )
        .with_state(DepositState::Accepted),
    ];

    (addresses, txids, deposits)
}

/// Creates an OP_RETURN metadata script.
pub fn create_opreturn_metadata(
    magic: [u8; 4],
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: &[u8; 32],
) -> ScriptBuf {
    let mut metadata = [0u8; 44];
    metadata[..4].copy_from_slice(&magic);
    // first 4 bytes = operator idx
    metadata[4..8].copy_from_slice(&operator_idx.to_be_bytes());
    // next 4 bytes = deposit idx
    metadata[8..12].copy_from_slice(&deposit_idx.to_be_bytes());
    metadata[12..44].copy_from_slice(deposit_txid);
    Descriptor::new_op_return(&metadata).unwrap().to_script()
}
