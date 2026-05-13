//! Replacement transaction builders.

use std::slice::from_ref;

use bitcoin::{
    hashes::Hash,
    key::Keypair,
    secp256k1::{schnorr::Signature, Message, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{ControlBlock, LeafVersion, TapLeafHash},
    Amount, FeeRate, ScriptBuf, Sequence, Transaction, TxOut, Txid,
};
use bitcoind_async_client::{error::ClientError, traits::Signer, types::PsbtBumpFeeOptions};
use strata_db_types::types::{
    L1TxId, Sighash, TerminalError, TxAttempt, TxAttemptStatus, TxNodeKind,
};
use strata_primitives::buf::Buf32;
use thiserror::Error;

use crate::writer::builder::BITCOIN_DUST_LIMIT;

/// Errors raised while building a replacement transaction.
#[derive(Debug, Error)]
pub enum ReplacementError {
    #[error("unsupported RBF transaction kind: {0:?}")]
    UnsupportedKind(TxNodeKind),
    #[error("Bitcoin wallet could not bump fee: {0}")]
    PsbtBumpFee(#[source] ClientError),
    #[error("Bitcoin wallet could not sign replacement PSBT: {0}")]
    WalletProcessPsbt(#[source] ClientError),
    #[error("wallet returned incomplete replacement PSBT")]
    IncompletePsbt,
    #[error("wallet returned no finalized replacement transaction")]
    MissingFinalTransaction,
    #[error("active reveal transaction is missing its tapscript witness")]
    MissingRevealWitness,
    #[error("invalid reveal control block: {0}")]
    InvalidControlBlock(String),
    #[error("replacement would reduce reveal output below dust")]
    ReplacementWouldDustOutput,
    #[error("failed to sign reveal replacement: {0}")]
    RevealSigning(#[source] anyhow::Error),
}

impl ReplacementError {
    /// Maps non-recoverable replacement failures to terminal node errors.
    pub fn terminal_error(&self) -> TerminalError {
        match self {
            Self::UnsupportedKind(_) => TerminalError::UnsupportedRbfKind,
            Self::PsbtBumpFee(ClientError::Server(_, msg))
                if msg.to_ascii_lowercase().contains("insufficient") =>
            {
                TerminalError::WalletInsufficient
            }
            Self::PsbtBumpFee(_) => TerminalError::Bip125FeeRuleUnsatisfiable,
            Self::WalletProcessPsbt(_) | Self::IncompletePsbt | Self::MissingFinalTransaction => {
                TerminalError::WalletInsufficient
            }
            Self::MissingRevealWitness | Self::InvalidControlBlock(_) | Self::RevealSigning(_) => {
                TerminalError::UnsupportedRbfKind
            }
            Self::ReplacementWouldDustOutput => TerminalError::ReplacementWouldDustOutput,
        }
    }
}

/// Builds a wallet-owned commit replacement using Bitcoin Core's wallet RBF flow.
pub async fn build_wallet_commit_replacement<C: Signer>(
    client: &C,
    kind: &TxNodeKind,
    active_txid: L1TxId,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> Result<TxAttempt, ReplacementError> {
    if !matches!(
        kind,
        TxNodeKind::SingleEnvelopeCommit { .. } | TxNodeKind::ChunkedEnvelopeCommit { .. }
    ) {
        return Err(ReplacementError::UnsupportedKind(kind.clone()));
    }

    let txid = Txid::from_byte_array(active_txid.0);
    let bumped = client
        .psbt_bump_fee(
            &txid,
            Some(PsbtBumpFeeOptions {
                fee_rate: Some(target_fee_rate),
                replaceable: Some(true),
                ..PsbtBumpFeeOptions::default()
            }),
        )
        .await
        .map_err(ReplacementError::PsbtBumpFee)?;

    let processed = client
        .wallet_process_psbt(&bumped.psbt.to_string(), Some(true), None, None)
        .await
        .map_err(ReplacementError::WalletProcessPsbt)?;
    if !processed.complete {
        return Err(ReplacementError::IncompletePsbt);
    }
    let tx: Transaction = processed
        .hex
        .ok_or(ReplacementError::MissingFinalTransaction)?;

    Ok(TxAttempt::new(
        &tx,
        target_fee_rate,
        Amount::from_sat(bumped.fee.to_sat()),
        attempt_no,
        TxAttemptStatus::Active,
    ))
}

/// Rebuilds and signs a chunked-envelope reveal by reducing its spendable output.
pub fn build_chunked_reveal_replacement(
    active_reveal_tx: &Transaction,
    commit_output: &TxOut,
    target_fee_rate: FeeRate,
    attempt_no: u32,
    sequencer_keypair: &Keypair,
) -> Result<TxAttempt, ReplacementError> {
    let mut replacement_tx = active_reveal_tx.clone();
    set_reveal_replacement_fee(&mut replacement_tx, commit_output, target_fee_rate)?;

    let (reveal_script, control_block) = extract_reveal_witness(active_reveal_tx)?;
    replacement_tx.input[0].witness.clear();
    let sighash =
        compute_taproot_script_spend_sighash(&replacement_tx, commit_output, &reveal_script)
            .map_err(ReplacementError::RevealSigning)?;
    let message = Message::from_digest_slice(sighash.as_ref())
        .map_err(|error| ReplacementError::RevealSigning(error.into()))?;
    let signature = SECP256K1.sign_schnorr(&message, sequencer_keypair);
    attach_reveal_witness(
        &mut replacement_tx,
        &reveal_script,
        &control_block,
        signature.as_ref(),
    )?;

    let fee = reveal_fee(&replacement_tx, commit_output);
    Ok(TxAttempt::new(
        &replacement_tx,
        target_fee_rate,
        fee,
        attempt_no,
        TxAttemptStatus::Active,
    ))
}

/// Rebuilds a single-envelope reveal with a higher fee, leaving it unsigned.
pub fn build_pending_single_reveal_replacement(
    active_reveal_tx: &Transaction,
    commit_output: &TxOut,
    target_fee_rate: FeeRate,
    attempt_no: u32,
) -> Result<(TxAttempt, Sighash), ReplacementError> {
    let mut replacement_tx = active_reveal_tx.clone();
    set_reveal_replacement_fee(&mut replacement_tx, commit_output, target_fee_rate)?;

    let (reveal_script, _) = extract_reveal_witness(active_reveal_tx)?;
    replacement_tx.input[0].witness.clear();
    let sighash =
        compute_taproot_script_spend_sighash(&replacement_tx, commit_output, &reveal_script)
            .map_err(ReplacementError::RevealSigning)?;
    let fee = reveal_fee(&replacement_tx, commit_output);

    Ok((
        TxAttempt::pending_signature(&replacement_tx, target_fee_rate, fee, attempt_no),
        Buf32(sighash),
    ))
}

/// Re-signs an existing chunked reveal so it spends a replacement commit output.
pub fn rebuild_reveal_for_replaced_commit(
    old_reveal_tx: &Transaction,
    replacement_commit_txid: Txid,
    replacement_commit_output: &TxOut,
    sequencer_keypair: &Keypair,
) -> Result<Transaction, ReplacementError> {
    let mut replacement_reveal = old_reveal_tx.clone();
    let input = replacement_reveal
        .input
        .first_mut()
        .ok_or(ReplacementError::MissingRevealWitness)?;
    input.previous_output.txid = replacement_commit_txid;
    input.sequence = Sequence::ENABLE_RBF_NO_LOCKTIME;

    let (reveal_script, control_block) = extract_reveal_witness(old_reveal_tx)?;
    replacement_reveal.input[0].witness.clear();
    let sighash = compute_taproot_script_spend_sighash(
        &replacement_reveal,
        replacement_commit_output,
        &reveal_script,
    )
    .map_err(ReplacementError::RevealSigning)?;
    let message = Message::from_digest_slice(sighash.as_ref())
        .map_err(|error| ReplacementError::RevealSigning(error.into()))?;
    let signature = SECP256K1.sign_schnorr(&message, sequencer_keypair);
    attach_reveal_witness(
        &mut replacement_reveal,
        &reveal_script,
        &control_block,
        signature.as_ref(),
    )?;

    Ok(replacement_reveal)
}

/// Rebuilds a single-envelope reveal for a replacement commit, leaving it unsigned.
pub fn rebuild_pending_reveal_for_replaced_commit(
    old_reveal_tx: &Transaction,
    replacement_commit_txid: Txid,
    replacement_commit_output: &TxOut,
    fee_rate: FeeRate,
    fee: Amount,
    attempt_no: u32,
) -> Result<(TxAttempt, Sighash), ReplacementError> {
    let mut replacement_reveal = old_reveal_tx.clone();
    let input = replacement_reveal
        .input
        .first_mut()
        .ok_or(ReplacementError::MissingRevealWitness)?;
    input.previous_output.txid = replacement_commit_txid;
    input.sequence = Sequence::ENABLE_RBF_NO_LOCKTIME;

    let (reveal_script, _) = extract_reveal_witness(old_reveal_tx)?;
    replacement_reveal.input[0].witness.clear();
    let sighash = compute_taproot_script_spend_sighash(
        &replacement_reveal,
        replacement_commit_output,
        &reveal_script,
    )
    .map_err(ReplacementError::RevealSigning)?;

    Ok((
        TxAttempt::pending_signature(&replacement_reveal, fee_rate, fee, attempt_no),
        Buf32(sighash),
    ))
}

pub(crate) fn extract_reveal_witness(
    tx: &Transaction,
) -> Result<(ScriptBuf, ControlBlock), ReplacementError> {
    let witness = tx
        .input
        .first()
        .ok_or(ReplacementError::MissingRevealWitness)?
        .witness
        .iter()
        .collect::<Vec<_>>();
    let reveal_script = witness
        .get(1)
        .ok_or(ReplacementError::MissingRevealWitness)?;
    let control_block = witness
        .get(2)
        .ok_or(ReplacementError::MissingRevealWitness)?;
    let control_block = ControlBlock::decode(control_block)
        .map_err(|error| ReplacementError::InvalidControlBlock(error.to_string()))?;
    Ok((ScriptBuf::from_bytes(reveal_script.to_vec()), control_block))
}

pub(crate) fn compute_taproot_script_spend_sighash(
    reveal_tx: &Transaction,
    output_to_reveal: &TxOut,
    reveal_script: &ScriptBuf,
) -> anyhow::Result<[u8; 32]> {
    let mut sighash_cache = SighashCache::new(reveal_tx);
    let signature_hash = sighash_cache.taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(from_ref(output_to_reveal)),
        TapLeafHash::from_script(reveal_script, LeafVersion::TapScript),
        TapSighashType::Default,
    )?;
    Ok(signature_hash.to_byte_array())
}

pub(crate) fn attach_reveal_witness(
    reveal_tx: &mut Transaction,
    reveal_script: &ScriptBuf,
    control_block: &ControlBlock,
    signature: &[u8; 64],
) -> Result<(), ReplacementError> {
    let signature = Signature::from_slice(signature).map_err(|error| {
        ReplacementError::RevealSigning(anyhow::anyhow!("invalid schnorr signature: {error}"))
    })?;
    let witness = &mut reveal_tx.input[0].witness;
    witness.push(signature.as_ref());
    witness.push(reveal_script);
    witness.push(control_block.serialize());
    Ok(())
}

fn set_reveal_replacement_fee(
    replacement_tx: &mut Transaction,
    commit_output: &TxOut,
    target_fee_rate: FeeRate,
) -> Result<(), ReplacementError> {
    if let Some(input) = replacement_tx.input.first_mut() {
        input.sequence = Sequence::ENABLE_RBF_NO_LOCKTIME;
    }

    let target_fee = target_fee_rate
        .fee_vb(replacement_tx.vsize() as u64)
        .ok_or(ReplacementError::ReplacementWouldDustOutput)?;
    let other_output_value = replacement_tx
        .output
        .iter()
        .take(replacement_tx.output.len().saturating_sub(1))
        .map(|output| output.value)
        .sum::<Amount>();
    let output_and_fee = other_output_value
        .checked_add(target_fee)
        .ok_or(ReplacementError::ReplacementWouldDustOutput)?;
    let replacement_output = replacement_tx
        .output
        .last_mut()
        .ok_or(ReplacementError::ReplacementWouldDustOutput)?;
    let new_output_value = commit_output
        .value
        .checked_sub(output_and_fee)
        .ok_or(ReplacementError::ReplacementWouldDustOutput)?;
    if new_output_value.to_sat() < BITCOIN_DUST_LIMIT {
        return Err(ReplacementError::ReplacementWouldDustOutput);
    }
    replacement_output.value = new_output_value;
    Ok(())
}

fn reveal_fee(reveal_tx: &Transaction, commit_output: &TxOut) -> Amount {
    let output_value = reveal_tx
        .output
        .iter()
        .map(|output| output.value)
        .sum::<Amount>();
    commit_output
        .value
        .checked_sub(output_value)
        .unwrap_or(Amount::ZERO)
}
