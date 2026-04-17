//! `post-ee-da-envelope`: broadcast one EE-DA commit transaction and its reveals.
//!
//! This is a functional-test helper. It can deliberately construct malformed
//! commit/reveal shapes that production builders should never emit.

use std::str::FromStr;

use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::{json::ListUnspentResultEntry, RpcApi};
use bitcoin::{
    absolute::LockTime,
    opcodes::all::OP_RETURN,
    script::Builder,
    secp256k1::{Keypair, Message, SecretKey, XOnlyPublicKey, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{
        LeafVersion, Signature as TaprootSignature, TapLeafHash, TaprootBuilder, TaprootSpendInfo,
    },
    transaction::Version,
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_l1_txfmt::MagicBytes;

use crate::{btc::new_bitcoind_client, constants::NETWORK, error::Error};

const COMMIT_MARKER_VERSION: u32 = 0;
const INVALID_COMMIT_MARKER_VERSION: u32 = 1;
const DEFAULT_SLOT_VALUE_SATS: u64 = 10_000;
const DEFAULT_COMMIT_FEE_SATS: u64 = 2_000;
const DEFAULT_REVEAL_FEE_SATS: u64 = 1_000;
const BITCOIN_DUST_LIMIT: u64 = 546;
const WRONG_SEQUENCER_SECRET: [u8; 32] = [2; 32];

/// Broadcast one EE-DA envelope (commit + reveals) to bitcoind.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "post-ee-da-envelope")]
pub struct PostEeDaEnvelopeArgs {
    /// bitcoind RPC URL (include `/wallet/<name>` for non-default wallets).
    #[argh(option)]
    pub bitcoind_url: String,

    /// bitcoind RPC username.
    #[argh(option)]
    pub rpc_user: String,

    /// bitcoind RPC password.
    #[argh(option)]
    pub rpc_password: String,

    /// EE-DA commit marker magic bytes (4 ASCII chars).
    #[argh(option)]
    pub magic_bytes: MagicBytes,

    /// hex-encoded sequencer secret key used to sign reveal script paths.
    #[argh(option)]
    pub sequencer_secret_key: HexSecretKey,

    /// hex-encoded raw DA chunk; repeat once per reveal slot.
    #[argh(option)]
    pub chunk_hex: Vec<HexBytes>,

    /// malformed envelope shape to post.
    #[argh(option, default = "MalformedEnvelopeMode::None")]
    pub malformed: MalformedEnvelopeMode,

    /// sats locked in each commit reveal slot.
    #[argh(option, default = "DEFAULT_SLOT_VALUE_SATS")]
    pub slot_value_sats: u64,

    /// flat commit transaction fee in sats.
    #[argh(option, default = "DEFAULT_COMMIT_FEE_SATS")]
    pub commit_fee_sats: u64,

    /// flat reveal transaction fee in sats.
    #[argh(option, default = "DEFAULT_REVEAL_FEE_SATS")]
    pub reveal_fee_sats: u64,
}

/// Malformed shapes that are useful for verifier functional tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedEnvelopeMode {
    /// Post a valid commit/reveal envelope.
    None,
    /// Use an unsupported commit marker version.
    UnsupportedVersion,
    /// Put the commit marker after output zero.
    MarkerAfterSlot,
    /// Add two commit marker outputs.
    MultipleMarkers,
    /// Include no P2TR reveal slots after the marker.
    MissingRevealSlots,
    /// Leave the final reveal slot unspent.
    MissingReveal,
    /// Spend two reveal slots from the same reveal transaction.
    MultiSlotReveal,
    /// Sign the reveal with a different key than the verifier config expects.
    WrongSequencerKey,
    /// Put bytes in the reveal that are not a valid encoded DA chunk.
    InvalidChunk,
    /// Add a P2TR change output after non-P2TR change.
    AmbiguousTaprootChange,
}

impl FromStr for MalformedEnvelopeMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "unsupported-version" => Ok(Self::UnsupportedVersion),
            "marker-after-slot" => Ok(Self::MarkerAfterSlot),
            "multiple-markers" => Ok(Self::MultipleMarkers),
            "missing-reveal-slots" => Ok(Self::MissingRevealSlots),
            "missing-reveal" => Ok(Self::MissingReveal),
            "multi-slot-reveal" => Ok(Self::MultiSlotReveal),
            "wrong-sequencer-key" => Ok(Self::WrongSequencerKey),
            "invalid-chunk" => Ok(Self::InvalidChunk),
            "ambiguous-taproot-change" => Ok(Self::AmbiguousTaprootChange),
            _ => Err(format!("unknown malformed envelope mode: {value}")),
        }
    }
}

/// Hex-encoded byte string (with optional `0x` prefix) parsed directly by argh.
#[derive(PartialEq, Eq, Debug, Clone)]
pub struct HexBytes(pub Vec<u8>);

impl FromStr for HexBytes {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s.trim_start_matches("0x")).map(HexBytes)
    }
}

/// Hex-encoded secp256k1 secret key.
#[derive(Debug, Clone)]
pub struct HexSecretKey(pub SecretKey);

impl PartialEq for HexSecretKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.secret_bytes() == other.0.secret_bytes()
    }
}

impl Eq for HexSecretKey {}

impl FromStr for HexSecretKey {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s.trim_start_matches("0x")).map_err(|err| err.to_string())?;
        SecretKey::from_slice(&bytes)
            .map(HexSecretKey)
            .map_err(|err| format!("invalid secp256k1 secret key: {err}"))
    }
}

struct RevealSlot {
    commit_txid: Txid,
    vout: u32,
    value: Amount,
    prevout_script: ScriptBuf,
    reveal_script: ScriptBuf,
    spend_info: TaprootSpendInfo,
    keypair: Keypair,
}

pub(crate) fn post_ee_da_envelope(args: PostEeDaEnvelopeArgs) -> Result<(), DisplayedError> {
    validate_args(&args)?;

    let client = new_bitcoind_client(
        &args.bitcoind_url,
        None,
        Some(&args.rpc_user),
        Some(&args.rpc_password),
    )
    .internal_error("create bitcoind rpc client")?;

    let change_addr = client
        .get_new_address(None, None)
        .internal_error("get_new_address for change")?
        .require_network(NETWORK)
        .internal_error("change address network mismatch")?;

    let mine_addr = client
        .get_new_address(None, None)
        .internal_error("mining address")?
        .require_network(NETWORK)
        .internal_error("mining address network mismatch")?;

    let utxos = client
        .list_unspent(None, None, None, None, None)
        .internal_error("list_unspent")?;

    let change_script = change_addr.script_pubkey();
    let chunks = prepare_chunks(&args);
    let commit_tx = build_commit_tx(&args, &chunks, &utxos, change_script.clone())
        .internal_error("build EE-DA commit transaction")?;

    let signed_commit = client
        .sign_raw_transaction_with_wallet(&commit_tx, None, None)
        .internal_error("sign commit")?
        .transaction()
        .internal_error("extract signed commit")?;
    let commit_txid = signed_commit.compute_txid();
    let reveal_txs = build_reveals_for_commit(&args, &chunks, &signed_commit, change_script)
        .internal_error("build EE-DA reveal transactions")?;
    client
        .send_raw_transaction(&signed_commit)
        .internal_error("broadcast commit")?;

    let mut reveal_txids = Vec::with_capacity(reveal_txs.len());
    for reveal in &reveal_txs {
        client
            .send_raw_transaction(reveal)
            .internal_error("broadcast reveal")?;
        reveal_txids.push(reveal.compute_txid());
    }

    client
        .generate_to_address(1, &mine_addr)
        .internal_error("mine EE-DA envelope")?;

    println!("commit_txid={commit_txid}");
    for reveal_txid in reveal_txids {
        println!("reveal_txid={reveal_txid}");
    }

    Ok(())
}

fn validate_args(args: &PostEeDaEnvelopeArgs) -> Result<(), DisplayedError> {
    if args.slot_value_sats <= args.reveal_fee_sats + BITCOIN_DUST_LIMIT {
        return Err(user_error(
            "invalid reveal slot value",
            format!(
                "--slot-value-sats ({}) must exceed --reveal-fee-sats ({}) plus dust ({BITCOIN_DUST_LIMIT})",
                args.slot_value_sats, args.reveal_fee_sats
            ),
        ));
    }

    match args.malformed {
        MalformedEnvelopeMode::MissingRevealSlots => Ok(()),
        MalformedEnvelopeMode::MultiSlotReveal if args.chunk_hex.len() < 2 => Err(user_error(
            "invalid malformed envelope arguments",
            "multi-slot-reveal requires at least two --chunk-hex values".to_string(),
        )),
        _ if args.chunk_hex.is_empty() => Err(user_error(
            "invalid malformed envelope arguments",
            "--chunk-hex must be provided at least once".to_string(),
        )),
        _ => Ok(()),
    }
}

fn user_error(context: &'static str, message: String) -> DisplayedError {
    DisplayedError::UserError(context.to_string(), Box::new(Error::TxBuilder(message)))
}

fn prepare_chunks(args: &PostEeDaEnvelopeArgs) -> Vec<Vec<u8>> {
    let mut chunks: Vec<Vec<u8>> = args.chunk_hex.iter().map(|chunk| chunk.0.clone()).collect();
    if args.malformed == MalformedEnvelopeMode::InvalidChunk {
        chunks[0] = vec![0xff; 16];
    }
    chunks
}

fn build_commit_tx(
    args: &PostEeDaEnvelopeArgs,
    chunks: &[Vec<u8>],
    utxos: &[ListUnspentResultEntry],
    change_script: ScriptBuf,
) -> Result<Transaction, Error> {
    let commit_outputs = build_commit_outputs(args, chunks, change_script.clone())?;
    let required_value = commit_outputs
        .iter()
        .map(|output| output.value.to_sat())
        .sum::<u64>()
        + args.commit_fee_sats;
    let funding_utxo = select_funding_utxo(utxos, required_value)?;
    let funding_value = funding_utxo.amount.to_sat();

    let mut outputs = commit_outputs;
    let change_value = funding_value
        .checked_sub(required_value)
        .ok_or_else(|| Error::TxBuilder("funding value below required value".to_string()))?;
    if change_value >= BITCOIN_DUST_LIMIT {
        outputs.push(TxOut {
            value: Amount::from_sat(change_value),
            script_pubkey: change_script.clone(),
        });
    }

    Ok(Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![make_txin(funding_utxo.txid, funding_utxo.vout)],
        output: outputs,
    })
}

fn build_commit_outputs(
    args: &PostEeDaEnvelopeArgs,
    chunks: &[Vec<u8>],
    change_script: ScriptBuf,
) -> Result<Vec<TxOut>, Error> {
    let mut outputs = Vec::new();
    let marker_version = match args.malformed {
        MalformedEnvelopeMode::UnsupportedVersion => INVALID_COMMIT_MARKER_VERSION,
        _ => COMMIT_MARKER_VERSION,
    };
    let slot_count = reveal_slot_count(args.malformed, chunks.len());
    let marker = TxOut {
        value: Amount::ZERO,
        script_pubkey: commit_marker_script(args.magic_bytes, marker_version)?,
    };

    if args.malformed == MalformedEnvelopeMode::MarkerAfterSlot {
        let (keypair, _) = reveal_keypair(args);
        outputs.push(reveal_output(args, &keypair, b"marker-after-slot")?);
        outputs.push(marker);
        return Ok(outputs);
    }

    outputs.push(marker);
    for chunk in chunks.iter().take(slot_count) {
        let (keypair, _) = reveal_keypair(args);
        outputs.push(reveal_output(args, &keypair, chunk)?);
    }

    if args.malformed == MalformedEnvelopeMode::MultipleMarkers {
        outputs.push(TxOut {
            value: Amount::ZERO,
            script_pubkey: commit_marker_script(args.magic_bytes, COMMIT_MARKER_VERSION)?,
        });
    }

    if args.malformed == MalformedEnvelopeMode::AmbiguousTaprootChange {
        outputs.push(TxOut {
            value: Amount::from_sat(BITCOIN_DUST_LIMIT),
            script_pubkey: change_script,
        });
        let (keypair, _) = reveal_keypair(args);
        outputs.push(reveal_output(args, &keypair, b"ambiguous-change")?);
    }

    Ok(outputs)
}

fn build_reveal_slots(
    args: &PostEeDaEnvelopeArgs,
    chunks: &[Vec<u8>],
    commit_tx: &Transaction,
) -> Result<Vec<RevealSlot>, Error> {
    if matches!(
        args.malformed,
        MalformedEnvelopeMode::UnsupportedVersion
            | MalformedEnvelopeMode::MarkerAfterSlot
            | MalformedEnvelopeMode::MultipleMarkers
            | MalformedEnvelopeMode::MissingRevealSlots
            | MalformedEnvelopeMode::AmbiguousTaprootChange
    ) {
        return Ok(Vec::new());
    }

    let slot_count = reveal_slot_count(args.malformed, chunks.len());
    let mut slots = Vec::with_capacity(slot_count);
    for (slot, chunk) in chunks.iter().take(slot_count).enumerate() {
        let (keypair, pubkey) = reveal_keypair(args);
        let reveal_script = reveal_script(pubkey, chunk)?;
        let spend_info = taproot_spend_info(pubkey, &reveal_script)?;
        let vout = (slot + 1) as u32;
        let prevout = commit_tx
            .output
            .get(vout as usize)
            .ok_or_else(|| Error::TxBuilder(format!("missing commit output {vout}")))?;
        slots.push(RevealSlot {
            commit_txid: commit_tx.compute_txid(),
            vout,
            value: prevout.value,
            prevout_script: prevout.script_pubkey.clone(),
            reveal_script,
            spend_info,
            keypair,
        });
    }
    Ok(slots)
}

fn build_reveals_for_commit(
    args: &PostEeDaEnvelopeArgs,
    chunks: &[Vec<u8>],
    commit_tx: &Transaction,
    change_script: ScriptBuf,
) -> Result<Vec<Transaction>, Error> {
    let slots = build_reveal_slots(args, chunks, commit_tx)?;
    build_reveal_txs(args, &slots, change_script)
}

fn reveal_slot_count(mode: MalformedEnvelopeMode, chunk_count: usize) -> usize {
    match mode {
        MalformedEnvelopeMode::MissingRevealSlots => 0,
        MalformedEnvelopeMode::MultiSlotReveal => 2,
        _ => chunk_count,
    }
}

fn build_reveal_txs(
    args: &PostEeDaEnvelopeArgs,
    slots: &[RevealSlot],
    change_script: ScriptBuf,
) -> Result<Vec<Transaction>, Error> {
    match args.malformed {
        MalformedEnvelopeMode::MissingReveal if !slots.is_empty() => slots[..slots.len() - 1]
            .iter()
            .map(|slot| build_single_reveal_tx(args, slot, change_script.clone()))
            .collect(),
        MalformedEnvelopeMode::MultiSlotReveal => Ok(vec![build_multi_slot_reveal_tx(
            args,
            slots,
            change_script,
        )?]),
        _ => slots
            .iter()
            .map(|slot| build_single_reveal_tx(args, slot, change_script.clone()))
            .collect(),
    }
}

fn build_single_reveal_tx(
    args: &PostEeDaEnvelopeArgs,
    slot: &RevealSlot,
    change_script: ScriptBuf,
) -> Result<Transaction, Error> {
    let mut tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![make_txin(slot.commit_txid, slot.vout)],
        output: vec![TxOut {
            value: Amount::from_sat(slot.value.to_sat() - args.reveal_fee_sats),
            script_pubkey: change_script,
        }],
    };
    let prevouts = vec![TxOut {
        value: slot.value,
        script_pubkey: slot.prevout_script.clone(),
    }];
    sign_reveal_input(&mut tx, 0, &prevouts, slot)?;
    Ok(tx)
}

fn build_multi_slot_reveal_tx(
    args: &PostEeDaEnvelopeArgs,
    slots: &[RevealSlot],
    change_script: ScriptBuf,
) -> Result<Transaction, Error> {
    let commit_txid = slots
        .first()
        .ok_or_else(|| Error::TxBuilder("multi-slot reveal requires slots".to_string()))?
        .commit_txid;
    let inputs = slots
        .iter()
        .map(|slot| make_txin(commit_txid, slot.vout))
        .collect();
    let prevouts = slots
        .iter()
        .map(|slot| TxOut {
            value: slot.value,
            script_pubkey: slot.prevout_script.clone(),
        })
        .collect::<Vec<_>>();
    let output_value =
        slots.iter().map(|slot| slot.value.to_sat()).sum::<u64>() - args.reveal_fee_sats;
    let mut tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: vec![TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: change_script,
        }],
    };
    for (input_index, slot) in slots.iter().enumerate() {
        sign_reveal_input(&mut tx, input_index, &prevouts, slot)?;
    }
    Ok(tx)
}

fn sign_reveal_input(
    tx: &mut Transaction,
    input_index: usize,
    prevouts: &[TxOut],
    slot: &RevealSlot,
) -> Result<(), Error> {
    let sighash = {
        let mut sighash_cache = SighashCache::new(&*tx);
        sighash_cache
            .taproot_script_spend_signature_hash(
                input_index,
                &Prevouts::All(prevouts),
                TapLeafHash::from_script(&slot.reveal_script, LeafVersion::TapScript),
                TapSighashType::Default,
            )
            .map_err(|err| Error::TxBuilder(format!("failed to compute reveal sighash: {err}")))?
    };
    let message = Message::from_digest_slice(sighash.as_ref())
        .map_err(|err| Error::TxBuilder(format!("invalid reveal sighash: {err}")))?;
    let signature = SECP256K1.sign_schnorr_no_aux_rand(&message, &slot.keypair);
    let signature = TaprootSignature {
        signature,
        sighash_type: TapSighashType::Default,
    };
    let control_block = slot
        .spend_info
        .control_block(&(slot.reveal_script.clone(), LeafVersion::TapScript))
        .ok_or_else(|| Error::TxBuilder("missing reveal control block".to_string()))?;

    let witness = &mut tx.input[input_index].witness;
    witness.push(signature.to_vec());
    witness.push(slot.reveal_script.clone());
    witness.push(control_block.serialize());
    Ok(())
}

fn commit_marker_script(magic_bytes: MagicBytes, version: u32) -> Result<ScriptBuf, Error> {
    let mut payload = [0u8; 8];
    payload[..4].copy_from_slice(magic_bytes.as_bytes());
    payload[4..].copy_from_slice(&version.to_be_bytes());
    Ok(Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(payload)
        .into_script())
}

fn reveal_output(
    args: &PostEeDaEnvelopeArgs,
    keypair: &Keypair,
    chunk: &[u8],
) -> Result<TxOut, Error> {
    let pubkey = XOnlyPublicKey::from_keypair(keypair).0;
    let script = reveal_script(pubkey, chunk)?;
    let spend_info = taproot_spend_info(pubkey, &script)?;
    Ok(TxOut {
        value: Amount::from_sat(args.slot_value_sats),
        script_pubkey: ScriptBuf::new_p2tr_tweaked(spend_info.output_key()),
    })
}

fn reveal_script(pubkey: XOnlyPublicKey, chunk: &[u8]) -> Result<ScriptBuf, Error> {
    EnvelopeScriptBuilder::with_pubkey(&pubkey.serialize())
        .map_err(|err| Error::TxBuilder(format!("invalid reveal pubkey: {err}")))?
        .add_envelope(chunk)
        .map_err(|err| Error::TxBuilder(format!("invalid reveal chunk: {err}")))?
        .build_without_min_check()
        .map_err(|err| Error::TxBuilder(format!("failed to build reveal script: {err}")))
}

fn taproot_spend_info(
    pubkey: XOnlyPublicKey,
    reveal_script: &ScriptBuf,
) -> Result<TaprootSpendInfo, Error> {
    TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .map_err(|err| Error::TxBuilder(format!("failed to add reveal leaf: {err}")))?
        .finalize(SECP256K1, pubkey)
        .map_err(|_| Error::TxBuilder("failed to finalize reveal taproot tree".to_string()))
}

fn reveal_keypair(args: &PostEeDaEnvelopeArgs) -> (Keypair, XOnlyPublicKey) {
    let secret_key = match args.malformed {
        MalformedEnvelopeMode::WrongSequencerKey => {
            SecretKey::from_slice(&WRONG_SEQUENCER_SECRET).expect("valid test secret key")
        }
        _ => args.sequencer_secret_key.0,
    };
    let keypair = Keypair::from_secret_key(SECP256K1, &secret_key);
    let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
    (keypair, pubkey)
}

fn select_funding_utxo(
    utxos: &[ListUnspentResultEntry],
    required_value: u64,
) -> Result<ListUnspentResultEntry, Error> {
    utxos
        .iter()
        .filter(|utxo| utxo.spendable && utxo.solvable && utxo.amount.to_sat() >= required_value)
        .min_by_key(|utxo| utxo.amount)
        .cloned()
        .ok_or_else(|| Error::TxBuilder(format!("insufficient funds: need {required_value} sats")))
}

fn make_txin(txid: Txid, vout: u32) -> TxIn {
    TxIn {
        previous_output: OutPoint { txid, vout },
        script_sig: ScriptBuf::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        witness: Witness::new(),
    }
}
