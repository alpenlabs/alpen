//! Replacement transaction builders.

use std::slice::from_ref;

use bitcoin::{
    hashes::Hash,
    key::Keypair,
    script::Instruction,
    secp256k1::{schnorr::Signature, Message, XOnlyPublicKey, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{ControlBlock, LeafVersion, TapLeafHash},
    Amount, FeeRate, ScriptBuf, Sequence, Transaction, TxOut, Txid,
};
use bitcoind_async_client::{error::ClientError, traits::Signer, types::PsbtBumpFeeOptions};
use strata_db_types::{
    common::L1TxId,
    fee_bump::{TerminalError, TxAttempt, TxAttemptStatus},
    l1_writer::Sighash,
};
use strata_primitives::buf::Buf32;
use thiserror::Error;

use crate::{
    rpc_error::is_retryable_client_error, tx_attempt::attempt_parts,
    writer::builder::BITCOIN_DUST_LIMIT,
};

/// Errors raised while building a replacement transaction.
#[derive(Debug, Error)]
pub(crate) enum ReplacementError {
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
    #[error("invalid reveal tapscript pubkey: {0}")]
    InvalidRevealPubkey(String),
    /// Keys are held as strings so the variant stays small: an inline [`XOnlyPublicKey`] pair is
    /// 128 bytes, which would grow every `Result<_, ReplacementError>` in this module past
    /// `clippy::result_large_err`.
    #[error("reveal tapscript commits to {committed} but the signer holds {held}")]
    RevealKeyRotated { committed: String, held: String },
    #[error("replacement would reduce reveal output below dust")]
    ReplacementWouldDustOutput,
    /// Discarded by the commit path rather than treated as terminal: which candidate the wallet
    /// builds depends on its UTXO set, so a later one can land under the ceiling.
    #[error("wallet built a replacement paying {built_sat_vb} sat/vB, above the configured ceiling of {ceiling_sat_vb} sat/vB")]
    ExceedsMaxFeeRate {
        built_sat_vb: u64,
        ceiling_sat_vb: u64,
    },
    #[error("replacement commit output layout is incompatible with the envelope: {0}")]
    IncompatibleCommitLayout(String),
    #[error("replacement would spend {0} wallet input(s) the original did not")]
    ReplacementAddsInputs(usize),
    #[error("failed to sign reveal replacement: {0}")]
    RevealSigning(#[source] anyhow::Error),
}

impl ReplacementError {
    /// Reports whether the failure is transient and the replacement is worth retrying.
    ///
    /// Wallet RPC calls fail for transport reasons too — bitcoind restarting, warming up, or
    /// briefly unreachable. Those must not burn the node's fee-bump chain, because a terminal
    /// error is permanent and would leave the transaction stuck at its original fee forever.
    pub(crate) fn is_retryable(&self) -> bool {
        match self {
            Self::PsbtBumpFee(error) | Self::WalletProcessPsbt(error) => {
                is_retryable_client_error(error)
            }
            // Not transient, but the commit path discards this candidate instead of ending the
            // chain; see the variant's own note.
            Self::ExceedsMaxFeeRate { .. } => false,
            Self::IncompletePsbt
            | Self::MissingFinalTransaction
            | Self::MissingRevealWitness
            | Self::InvalidControlBlock(_)
            | Self::InvalidRevealPubkey(_)
            | Self::RevealKeyRotated { .. }
            | Self::ReplacementWouldDustOutput
            | Self::IncompatibleCommitLayout(_)
            | Self::ReplacementAddsInputs(_)
            | Self::RevealSigning(_) => false,
        }
    }

    /// Maps non-recoverable replacement failures to terminal node errors.
    ///
    /// Only call this after [`ReplacementError::is_retryable`] has returned `false`.
    pub(crate) fn terminal_error(&self) -> TerminalError {
        match self {
            Self::PsbtBumpFee(ClientError::Server(_, msg))
                if msg.to_ascii_lowercase().contains("insufficient") =>
            {
                TerminalError::WalletInsufficient
            }
            Self::PsbtBumpFee(_) => TerminalError::Bip125FeeRuleUnsatisfiable,
            Self::WalletProcessPsbt(_) | Self::IncompletePsbt | Self::MissingFinalTransaction => {
                TerminalError::WalletInsufficient
            }
            Self::MissingRevealWitness
            | Self::InvalidControlBlock(_)
            | Self::InvalidRevealPubkey(_)
            | Self::RevealKeyRotated { .. }
            | Self::RevealSigning(_)
            | Self::IncompatibleCommitLayout(_) => TerminalError::UnsupportedRbfKind,
            Self::ReplacementAddsInputs(_) => TerminalError::ReplacementAddsInputs,
            Self::ReplacementWouldDustOutput => TerminalError::ReplacementWouldDustOutput,
            Self::ExceedsMaxFeeRate { .. } => TerminalError::AboveMaxFeeRate,
        }
    }
}

/// Builds a chunked-envelope commit replacement using Bitcoin Core's wallet RBF flow.
///
/// # Coin selection
///
/// When the original change output cannot absorb the higher fee, Core's fee bumper pulls in
/// additional confirmed wallet UTXOs. `psbtbumpfee` returns a PSBT rather than committing it, so
/// those inputs stay unlocked and keep appearing in `listunspent` until the replacement is
/// broadcast — a writer building another envelope in that window could select the same UTXO, and
/// whichever transaction Core saw first would invalidate the other.
///
/// Reserving those inputs properly needs `lockunspent`, which `bitcoind-async-client` does not
/// expose, plus durable bookkeeping to release the locks when a replacement is discarded. Until
/// then this fails closed: a replacement that spends any input the original did not is rejected.
/// Change-funded bumps — the common case — are unaffected. The cost is that a bump needing extra
/// inputs cannot proceed, which is a liveness limit rather than a correctness hazard.
///
/// `original_change_index` names the output Core should recycle to pay the higher fee. Passing it
/// is not an optimisation: see [`chunked_commit_change_index`] for why leaving Core to find the
/// change itself would make every chunked commit bump fail.
///
/// `max_fee_rate` is the operator's configured replacement ceiling. `target_fee_rate` is already
/// capped by it, but the wallet prices the fee off the transaction it ends up building and can pay
/// more than it was asked for, so the built candidate is checked against the ceiling too.
pub(crate) async fn build_chunked_commit_replacement<C: Signer>(
    client: &C,
    original_tx: &Transaction,
    active_txid: L1TxId,
    target_fee_rate: FeeRate,
    max_fee_rate: FeeRate,
    attempt_no: u32,
    original_change_index: Option<u32>,
) -> Result<TxAttempt, ReplacementError> {
    let txid = Txid::from_byte_array(active_txid.0);
    let bumped = client
        .psbt_bump_fee(
            &txid,
            Some(PsbtBumpFeeOptions {
                fee_rate: Some(target_fee_rate),
                replaceable: Some(true),
                original_change_index,
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

    reject_added_inputs(original_tx, &tx)?;

    // Record what the wallet actually built, not what it was asked for. Core sizes the fee from
    // the final transaction and can absorb sub-dust change into it, so the replacement often pays
    // more than `target_fee_rate`. The next bump derives its additive and multiplicative floors
    // from this number, and a target below the fee rate already on the wire is one Core rejects.
    let fee = Amount::from_sat(bumped.fee.to_sat());
    let effective_fee_rate = effective_fee_rate(&tx, fee).unwrap_or(target_fee_rate);
    let recorded_fee_rate = effective_fee_rate.max(target_fee_rate);

    // `target_fee_rate` already clears the ceiling, but what the wallet built need not: absorbing a
    // sub-dust change output into the fee raises the rate above what was asked for. Adopting it
    // anyway would put a transaction on the wire that breaches the operator's safety cap, and would
    // also seed the next bump's floors from a rate the ceiling forbids.
    if recorded_fee_rate > max_fee_rate {
        return Err(ReplacementError::ExceedsMaxFeeRate {
            built_sat_vb: recorded_fee_rate.to_sat_per_vb_ceil(),
            ceiling_sat_vb: max_fee_rate.to_sat_per_vb_ceil(),
        });
    }

    Ok(TxAttempt::new(
        attempt_parts(&tx, recorded_fee_rate, fee),
        attempt_no,
        TxAttemptStatus::Active,
    ))
}

/// Derives the fee rate a built transaction actually pays, rounded up.
///
/// `None` when the rate does not fit a [`FeeRate`], which leaves the caller with the target it
/// asked for rather than no attempt at all.
fn effective_fee_rate(tx: &Transaction, fee: Amount) -> Option<FeeRate> {
    let vsize = tx.vsize() as u64;
    if vsize == 0 {
        return None;
    }
    FeeRate::from_sat_per_vb(fee.to_sat().div_ceil(vsize))
}

/// Locates the change output Core should recycle when bumping a chunked commit.
///
/// Core only auto-detects an output as change when it is `IsMine` *and* absent from the wallet's
/// address book. The commit pays its change to the sequencer address, which both binaries obtain
/// from `getnewaddress`, and that writes an address-book entry. Left to its own detection Core
/// therefore treats every output as a fixed recipient, has nothing it is allowed to shrink, and can
/// only raise the fee by pulling in new inputs — which [`reject_added_inputs`] refuses, ending the
/// chain terminal on the very first bump. Naming the index sidesteps the heuristic entirely.
///
/// The layout is `[OP_RETURN, one funding output per reveal, change?]`; the builder appends change
/// last and only when the excess clears dust. `None` means there is no change output, and a bump
/// then genuinely does need new inputs (known gap 1).
pub(crate) fn chunked_commit_change_index(
    commit_tx: &Transaction,
    reveal_count: usize,
) -> Option<u32> {
    let change_index = reveal_count + 1;
    if commit_tx.output.len() > change_index {
        u32::try_from(change_index).ok()
    } else {
        None
    }
}

/// Checks that a replacement chunked commit still has the output layout the envelope requires.
///
/// The ASM guest infers the reveal count from the run of consecutive P2TR outputs that follows
/// the commit `OP_RETURN`, and each reveal is funded by exactly one of those outputs. Bitcoin
/// Core's `psbtbumpfee` is free to reduce, drop, or append a change output; if the wallet's change
/// type is taproot, an appended change output would extend that run and make the guest expect a
/// reveal that does not exist. It may also shrink a reveal-funding output below what its reveal
/// needs. Neither is recoverable, so verify the layout before adopting the replacement.
pub(crate) fn validate_chunked_commit_replacement_layout(
    original_commit: &Transaction,
    replacement_commit: &Transaction,
    reveal_count: usize,
) -> Result<(), ReplacementError> {
    let expected_prefix = reveal_count + 1;

    if original_commit.output.len() < expected_prefix {
        return Err(ReplacementError::IncompatibleCommitLayout(format!(
            "original commit has {} outputs, expected at least {expected_prefix}",
            original_commit.output.len()
        )));
    }
    if replacement_commit.output.len() < expected_prefix {
        return Err(ReplacementError::IncompatibleCommitLayout(format!(
            "replacement commit has {} outputs, expected at least {expected_prefix}",
            replacement_commit.output.len()
        )));
    }

    let op_return = &replacement_commit.output[0];
    if op_return.script_pubkey != original_commit.output[0].script_pubkey {
        return Err(ReplacementError::IncompatibleCommitLayout(
            "replacement commit changed the OP_RETURN output".to_string(),
        ));
    }

    // Reveal-funding outputs must survive byte-identical: their values size each reveal's fee.
    for vout in 1..expected_prefix {
        let original = &original_commit.output[vout];
        let replacement = &replacement_commit.output[vout];
        if original.script_pubkey != replacement.script_pubkey
            || original.value != replacement.value
        {
            return Err(ReplacementError::IncompatibleCommitLayout(format!(
                "replacement commit altered reveal-funding output {vout}"
            )));
        }
    }

    // Anything past the reveal outputs is change, and must not extend the P2TR run.
    for (offset, output) in replacement_commit.output[expected_prefix..]
        .iter()
        .enumerate()
    {
        if output.script_pubkey.is_p2tr() {
            return Err(ReplacementError::IncompatibleCommitLayout(format!(
                "replacement commit added a P2TR output at index {}, which would extend the reveal run",
                expected_prefix + offset
            )));
        }
    }

    Ok(())
}

/// Rejects a replacement that spends wallet inputs the original transaction did not.
///
/// See the coin-selection note on [`build_chunked_commit_replacement`].
fn reject_added_inputs(
    original_tx: &Transaction,
    replacement_tx: &Transaction,
) -> Result<(), ReplacementError> {
    let original_outpoints: Vec<_> = original_tx
        .input
        .iter()
        .map(|input| input.previous_output)
        .collect();
    let added = replacement_tx
        .input
        .iter()
        .filter(|input| !original_outpoints.contains(&input.previous_output))
        .count();

    if added > 0 {
        return Err(ReplacementError::ReplacementAddsInputs(added));
    }
    Ok(())
}

/// Verifies the sequencer still holds the key `reveal_tx`'s tapscript commits to.
///
/// A replacement reuses the original tapscript, so a signature under any other key yields a witness
/// that can never satisfy it — and by the time bitcoind says so the original is already marked
/// `Replaced`, leaving the envelope stuck with nothing to rebuild it. The single-envelope path
/// makes the same check against its external signer's key.
pub(crate) fn ensure_reveal_signable(
    reveal_tx: &Transaction,
    sequencer_keypair: &Keypair,
) -> Result<(), ReplacementError> {
    let committed = extract_reveal_pubkey(reveal_tx)?;
    let held = XOnlyPublicKey::from_keypair(sequencer_keypair).0;
    if committed != held {
        return Err(ReplacementError::RevealKeyRotated {
            committed: committed.to_string(),
            held: held.to_string(),
        });
    }
    Ok(())
}

/// Rebuilds and signs a chunked-envelope reveal by reducing its spendable output.
pub(crate) fn build_chunked_reveal_replacement(
    active_reveal_tx: &Transaction,
    commit_output: &TxOut,
    target_fee_rate: FeeRate,
    attempt_no: u32,
    sequencer_keypair: &Keypair,
) -> Result<TxAttempt, ReplacementError> {
    ensure_reveal_signable(active_reveal_tx, sequencer_keypair)?;

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
        attempt_parts(&replacement_tx, target_fee_rate, fee),
        attempt_no,
        TxAttemptStatus::Active,
    ))
}

/// Rebuilds a single-envelope reveal with a higher fee, leaving it unsigned.
pub(crate) fn build_pending_single_reveal_replacement(
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
        TxAttempt::pending_signature(
            attempt_parts(&replacement_tx, target_fee_rate, fee),
            attempt_no,
        ),
        Buf32(sighash),
    ))
}

/// Re-signs an existing chunked reveal so it spends a replacement commit output.
pub(crate) fn rebuild_reveal_for_replaced_commit(
    old_reveal_tx: &Transaction,
    replacement_commit_txid: Txid,
    replacement_commit_output: &TxOut,
    sequencer_keypair: &Keypair,
) -> Result<Transaction, ReplacementError> {
    ensure_reveal_signable(old_reveal_tx, sequencer_keypair)?;

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

/// Returns the x-only key committed in an active reveal's tapscript.
///
/// SPS-51 reveal scripts open with `<pubkey> OP_CHECKSIG`, so the leading push is the key any
/// witness signature must verify against. A replacement reuses the original script, which makes
/// this the key the signer has to still hold.
pub(crate) fn extract_reveal_pubkey(tx: &Transaction) -> Result<XOnlyPublicKey, ReplacementError> {
    let (reveal_script, _) = extract_reveal_witness(tx)?;
    reveal_script_pubkey(&reveal_script)
}

/// Returns the x-only key a reveal tapscript commits to.
pub(crate) fn reveal_script_pubkey(
    reveal_script: &ScriptBuf,
) -> Result<XOnlyPublicKey, ReplacementError> {
    let Some(Ok(Instruction::PushBytes(pubkey))) = reveal_script.instructions().next() else {
        return Err(ReplacementError::MissingRevealWitness);
    };
    XOnlyPublicKey::from_slice(pubkey.as_bytes())
        .map_err(|error| ReplacementError::InvalidRevealPubkey(error.to_string()))
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
    let witness = &mut reveal_tx
        .input
        .first_mut()
        .ok_or(ReplacementError::MissingRevealWitness)?
        .witness;
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

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use bitcoin::{
        absolute::LockTime, opcodes, script::Builder as ScriptBuilder, transaction::Version,
        OutPoint, TxIn, Witness, WitnessProgram, WitnessVersion,
    };
    use strata_config::btcio::FeeBumpingConfig;
    use strata_db_types::fee_bump::{TxNodeKind, TxNodeRecord};

    use super::*;
    use crate::{
        broadcaster::fee_bump::{
            evaluate_fee_bump, FeeBumpDecision, FeeBumpEvaluation, FeeBumpRequest,
        },
        test_utils::TestBitcoinClient,
        tx_attempt::TxAttemptExt,
        writer::builder::reveal_fee_headroom,
    };

    fn op_return_output() -> TxOut {
        TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuilder::new()
                .push_opcode(opcodes::all::OP_RETURN)
                .push_slice([1u8; 8])
                .into_script(),
        }
    }

    /// Builds a P2TR output directly from a witness program so tests need no valid curve point.
    fn p2tr_output(value: u64, seed: u8) -> TxOut {
        let program = WitnessProgram::new(WitnessVersion::V1, &[seed; 32]).expect("valid program");
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: ScriptBuf::new_witness_program(&program),
        }
    }

    fn p2wpkh_output(value: u64) -> TxOut {
        TxOut {
            value: Amount::from_sat(value),
            script_pubkey: ScriptBuf::new_p2wpkh(&bitcoin::WPubkeyHash::from_byte_array([7u8; 20])),
        }
    }

    fn test_keypair(seed: u8) -> Keypair {
        Keypair::from_seckey_slice(SECP256K1, &[seed; 32]).expect("valid secret key")
    }

    fn test_pubkey(seed: u8) -> XOnlyPublicKey {
        XOnlyPublicKey::from_keypair(&test_keypair(seed)).0
    }

    /// Builds a reveal spending a tapscript that commits to `pubkey`, as SPS-51 reveals do.
    fn reveal_committing_to(pubkey: &XOnlyPublicKey) -> Transaction {
        let reveal_script = ScriptBuilder::new()
            .push_slice(pubkey.serialize())
            .push_opcode(opcodes::all::OP_CHECKSIG)
            .into_script();

        let mut control_block = vec![LeafVersion::TapScript.to_consensus()];
        control_block.extend_from_slice(&pubkey.serialize());

        let mut witness = Witness::new();
        witness.push([0u8; 64]);
        witness.push(reveal_script.as_bytes());
        witness.push(&control_block);

        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness,
            }],
            output: vec![p2tr_output(5_000, 1)],
        }
    }

    #[test]
    fn extracts_the_key_committed_in_the_reveal_script() {
        let pubkey = test_pubkey(1);

        assert_eq!(
            extract_reveal_pubkey(&reveal_committing_to(&pubkey)).expect("pubkey is recoverable"),
            pubkey
        );
    }

    /// The rotation guard depends on this: a reveal built under one key must not compare equal to
    /// a different signer key.
    #[test]
    fn extracted_key_differs_across_reveals_built_under_different_keys() {
        let extracted = extract_reveal_pubkey(&reveal_committing_to(&test_pubkey(1)))
            .expect("pubkey is recoverable");

        assert_ne!(extracted, test_pubkey(2));
    }

    #[test]
    fn accepts_a_reveal_the_signer_still_holds_the_key_for() {
        let reveal = reveal_committing_to(&test_pubkey(1));

        assert!(ensure_reveal_signable(&reveal, &test_keypair(1)).is_ok());
    }

    /// Regression: a replacement reuses the original tapscript, so signing it under a rotated key
    /// yields a witness that can never satisfy that script — and the original is marked `Replaced`
    /// before bitcoind ever says so. Refusing is what leaves the envelope for the writer to
    /// rebuild.
    #[test]
    fn refuses_a_reveal_whose_committed_key_has_rotated() {
        let reveal = reveal_committing_to(&test_pubkey(1));

        let error = ensure_reveal_signable(&reveal, &test_keypair(2))
            .expect_err("a rotated key must not be signed with");

        assert!(matches!(error, ReplacementError::RevealKeyRotated { .. }));
        assert!(!error.is_retryable());
        assert_eq!(error.terminal_error(), TerminalError::UnsupportedRbfKind);
    }

    /// A production-shaped reveal uses build-time headroom to pay a valid replacement fee.
    #[test]
    fn a_production_shaped_reveal_with_headroom_bumps_successfully() {
        let build_rate = FeeRate::from_sat_per_vb(2).expect("valid fee rate");
        let target_rate = FeeRate::from_sat_per_vb(3).expect("valid fee rate");
        let keypair = test_keypair(1);
        let mut reveal = reveal_committing_to(&test_pubkey(1));
        let reveal_vsize = u64::try_from(reveal.vsize()).expect("vsize fits");
        let base_fee = build_rate.fee_vb(reveal_vsize).expect("reveal fee fits");
        let headroom = reveal_fee_headroom(build_rate, reveal_vsize, &FeeBumpingConfig::default())
            .expect("headroom fits");
        reveal.output = vec![p2tr_output(BITCOIN_DUST_LIMIT + headroom, 1)];
        let commit_value = reveal.output[0].value + base_fee;
        let commit_output = TxOut {
            value: commit_value,
            script_pubkey: reveal.output[0].script_pubkey.clone(),
        };

        let replacement =
            build_chunked_reveal_replacement(&reveal, &commit_output, target_rate, 1, &keypair)
                .expect("headroom funds the replacement");
        let replacement_tx = replacement.try_to_tx().expect("replacement decodes");
        let target_fee = target_rate
            .fee_vb(u64::try_from(replacement_tx.vsize()).expect("vsize fits"))
            .expect("target fee fits");
        let actual_fee = reveal_fee(&replacement_tx, &commit_output);
        let (reveal_script, _) = extract_reveal_witness(&replacement_tx).expect("witness parses");
        let sighash =
            compute_taproot_script_spend_sighash(&replacement_tx, &commit_output, &reveal_script)
                .expect("sighash computes");
        let message = Message::from_digest_slice(&sighash).expect("message parses");
        let signature = Signature::from_slice(
            replacement_tx.input[0]
                .witness
                .iter()
                .next()
                .expect("signature witness"),
        )
        .expect("signature parses");

        assert_eq!(actual_fee, target_fee);
        assert!(replacement_tx.output.last().unwrap().value.to_sat() >= BITCOIN_DUST_LIMIT);
        SECP256K1
            .verify_schnorr(&signature, &message, &test_pubkey(1))
            .expect("replacement witness verifies under the sequencer key");
    }

    #[test]
    fn every_policy_replacement_fits_the_reveal_builder_budget() {
        let keypair = test_keypair(1);
        let reveal_kind = TxNodeKind::ChunkedEnvelopeReveal {
            envelope_idx: 0,
            reveal_idx: 0,
        };

        for extra_output_count in 0..=2 {
            for case in [
                "fundable_boundary",
                "above_fundable_boundary",
                "fractional_build_rate",
                "raised_incremental_relay_fee",
                "estimator_jump",
                "headroom_cap",
                "build_rate_at_max",
            ] {
                let default_config = FeeBumpingConfig::default();
                let config = if case == "headroom_cap" {
                    FeeBumpingConfig {
                        max_reveal_fee_headroom_sats: NonZeroU64::new(7).unwrap(),
                        ..default_config
                    }
                } else {
                    default_config
                };
                let build_rate = if case == "fractional_build_rate" {
                    FeeRate::from_sat_per_kwu(125)
                } else if case == "build_rate_at_max" {
                    config.max_fee_rate()
                } else {
                    FeeRate::from_sat_per_vb(1).unwrap()
                };
                let mut reveal = reveal_committing_to(&test_pubkey(1));
                for _ in 0..extra_output_count {
                    reveal.output.insert(0, op_return_output());
                }
                let reveal_vsize = u64::try_from(reveal.vsize()).expect("vsize fits");
                let base_fee = build_rate.fee_vb(reveal_vsize).expect("base fee fits");
                let headroom = reveal_fee_headroom(build_rate, reveal_vsize, &config)
                    .expect("headroom derives");
                reveal.output.last_mut().unwrap().value =
                    Amount::from_sat(BITCOIN_DUST_LIMIT + headroom);
                let other_output_value = reveal
                    .output
                    .iter()
                    .take(reveal.output.len() - 1)
                    .map(|output| output.value)
                    .sum::<Amount>();
                let commit_output = TxOut {
                    value: other_output_value + reveal.output.last().unwrap().value + base_fee,
                    script_pubkey: reveal.output.last().unwrap().script_pubkey.clone(),
                };
                let reveal_fee_budget = commit_output
                    .value
                    .checked_sub(other_output_value)
                    .and_then(|remaining| {
                        remaining.checked_sub(Amount::from_sat(BITCOIN_DUST_LIMIT))
                    })
                    .expect("budget is funded");
                let fundable_rate =
                    FeeRate::from_sat_per_vb(reveal_fee_budget.to_sat() / reveal_vsize)
                        .expect("fundable rate fits");
                let estimate_fee_rate = match case {
                    "above_fundable_boundary" => FeeRate::from_sat_per_vb(
                        fundable_rate.to_sat_per_vb_ceil().saturating_add(1),
                    )
                    .unwrap(),
                    "estimator_jump" => FeeRate::from_sat_per_vb(50_000).unwrap(),
                    _ => fundable_rate,
                };
                let incremental_relay_fee_rate = if case == "raised_incremental_relay_fee" {
                    FeeRate::from_sat_per_vb(3).unwrap()
                } else {
                    FeeRate::from_sat_per_vb(1).unwrap()
                };
                let mut active_attempt =
                    TxAttempt::active(attempt_parts(&reveal, build_rate, base_fee), 0);
                active_attempt.first_published_l1_height = Some(100);
                let record = TxNodeRecord::new(reveal_kind.clone(), active_attempt);
                let decision = evaluate_fee_bump(
                    &config,
                    &record,
                    record.active_attempt().unwrap(),
                    FeeBumpEvaluation {
                        current_l1_tip: 102,
                        estimate_fee_rate,
                        incremental_relay_fee_rate,
                        replacement_vsize: reveal.vsize(),
                        reveal_fee_budget: Some(reveal_fee_budget),
                    },
                );

                match decision {
                    FeeBumpDecision::Replace(FeeBumpRequest {
                        target_fee_rate, ..
                    }) => {
                        let mut direct_replacement = reveal.clone();
                        set_reveal_replacement_fee(
                            &mut direct_replacement,
                            &commit_output,
                            target_fee_rate,
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "policy returned Replace rejected by direct builder for {case} with {extra_output_count} extra outputs: {error}"
                            )
                        });
                        let chunked_replacement = build_chunked_reveal_replacement(
                            &reveal,
                            &commit_output,
                            target_fee_rate,
                            1,
                            &keypair,
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "policy returned Replace rejected by chunked builder for {case} with {extra_output_count} extra outputs: {error}"
                            )
                        })
                        .try_to_tx()
                        .expect("replacement decodes");

                        assert!(
                            direct_replacement.output.last().unwrap().value.to_sat()
                                >= BITCOIN_DUST_LIMIT
                        );
                        assert!(
                            chunked_replacement.output.last().unwrap().value.to_sat()
                                >= BITCOIN_DUST_LIMIT
                        );
                    }
                    FeeBumpDecision::BlockedByCeiling { ceiling, .. } => {
                        assert!(matches!(case, "above_fundable_boundary" | "estimator_jump"));
                        assert_eq!(ceiling, fundable_rate.min(config.max_fee_rate()));
                    }
                    FeeBumpDecision::Terminal(error) => match case {
                        "headroom_cap" => {
                            assert_eq!(error, TerminalError::RevealFeeHeadroomExhausted)
                        }
                        "build_rate_at_max" => {
                            assert_eq!(error, TerminalError::Bip125FeeRuleUnsatisfiable)
                        }
                        _ => panic!("unexpected terminal decision for {case}: {error}"),
                    },
                    FeeBumpDecision::Wait => panic!("stale reveal unexpectedly waited for {case}"),
                }
            }
        }
    }

    #[test]
    fn chunked_reveal_replacement_refuses_a_rotated_key() {
        let error = build_chunked_reveal_replacement(
            &reveal_committing_to(&test_pubkey(1)),
            &p2tr_output(10_000, 2),
            FeeRate::from_sat_per_vb(4).expect("valid fee rate"),
            1,
            &test_keypair(2),
        )
        .expect_err("a rotated key must not be signed with");

        assert!(matches!(error, ReplacementError::RevealKeyRotated { .. }));
    }

    /// The commit path re-signs every reveal of the envelope, so the same guard has to hold there:
    /// one rotated reveal would otherwise take the whole envelope down with it.
    #[test]
    fn commit_replacement_reveal_rebuild_refuses_a_rotated_key() {
        let error = rebuild_reveal_for_replaced_commit(
            &reveal_committing_to(&test_pubkey(1)),
            Txid::from_byte_array([3u8; 32]),
            &p2tr_output(10_000, 2),
            &test_keypair(2),
        )
        .expect_err("a rotated key must not be signed with");

        assert!(matches!(error, ReplacementError::RevealKeyRotated { .. }));
    }

    #[test]
    fn rejects_a_reveal_whose_script_does_not_open_with_a_key() {
        let mut tx = reveal_committing_to(&test_pubkey(1));
        let control_block = tx.input[0]
            .witness
            .iter()
            .nth(2)
            .expect("control block")
            .to_vec();
        let script = ScriptBuilder::new()
            .push_opcode(opcodes::all::OP_CHECKSIG)
            .into_script();

        let mut witness = Witness::new();
        witness.push([0u8; 64]);
        witness.push(script.as_bytes());
        witness.push(&control_block);
        tx.input[0].witness = witness;

        assert!(matches!(
            extract_reveal_pubkey(&tx),
            Err(ReplacementError::MissingRevealWitness)
        ));
    }

    #[test]
    fn refuses_to_attach_a_witness_to_a_reveal_without_inputs() {
        let reveal = reveal_committing_to(&test_pubkey(1));
        let (reveal_script, control_block) =
            extract_reveal_witness(&reveal).expect("test reveal has a witness");
        let mut empty_reveal = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        let error = attach_reveal_witness(
            &mut empty_reveal,
            &reveal_script,
            &control_block,
            &[1u8; 64],
        )
        .expect_err("a reveal without inputs cannot carry a witness");

        assert!(matches!(error, ReplacementError::MissingRevealWitness));
    }

    fn commit_with(outputs: Vec<TxOut>) -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
                witness: Witness::new(),
            }],
            output: outputs,
        }
    }

    /// Two reveals: `[OP_RETURN, P2TR, P2TR]` plus an optional change output.
    fn original_commit() -> Transaction {
        commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(6_000, 2),
            p2wpkh_output(20_000),
        ])
    }

    /// Core's own change detection ignores anything in the wallet's address book, which is where
    /// the sequencer address lives, so the index has to be supplied or every bump adds inputs and
    /// is refused.
    #[test]
    fn change_index_points_past_the_reveal_funding_run() {
        assert_eq!(chunked_commit_change_index(&original_commit(), 2), Some(3));
    }

    #[test]
    fn no_change_index_when_the_commit_has_no_change_output() {
        let commit = commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(6_000, 2),
        ]);

        assert_eq!(chunked_commit_change_index(&commit, 2), None);
    }

    #[test]
    fn accepts_replacement_that_only_shrinks_change() {
        let replacement = commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(6_000, 2),
            p2wpkh_output(18_000),
        ]);

        assert!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2).is_ok()
        );
    }

    #[test]
    fn accepts_replacement_that_drops_change_entirely() {
        let replacement = commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(6_000, 2),
        ]);

        assert!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2).is_ok()
        );
    }

    /// A taproot change output would extend the P2TR run the ASM guest uses to count reveals.
    #[test]
    fn rejects_replacement_with_p2tr_change() {
        let replacement = commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(6_000, 2),
            p2tr_output(18_000, 3),
        ]);

        assert!(matches!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2),
            Err(ReplacementError::IncompatibleCommitLayout(_))
        ));
    }

    #[test]
    fn rejects_replacement_that_shrinks_a_reveal_funding_output() {
        let replacement = commit_with(vec![
            op_return_output(),
            p2tr_output(5_000, 1),
            p2tr_output(4_000, 2),
            p2wpkh_output(20_000),
        ]);

        assert!(matches!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2),
            Err(ReplacementError::IncompatibleCommitLayout(_))
        ));
    }

    #[test]
    fn rejects_replacement_that_drops_a_reveal_funding_output() {
        let replacement = commit_with(vec![op_return_output(), p2tr_output(5_000, 1)]);

        assert!(matches!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2),
            Err(ReplacementError::IncompatibleCommitLayout(_))
        ));
    }

    #[test]
    fn rejects_replacement_that_changes_the_op_return() {
        let mut replacement = original_commit();
        replacement.output[0].script_pubkey = ScriptBuilder::new()
            .push_opcode(opcodes::all::OP_RETURN)
            .push_slice([9u8; 8])
            .into_script();

        assert!(matches!(
            validate_chunked_commit_replacement_layout(&original_commit(), &replacement, 2),
            Err(ReplacementError::IncompatibleCommitLayout(_))
        ));
    }

    #[test]
    fn incompatible_commit_layout_is_not_retryable_and_is_terminal() {
        let error = ReplacementError::IncompatibleCommitLayout("test".to_string());

        assert!(!error.is_retryable());
        assert_eq!(error.terminal_error(), TerminalError::UnsupportedRbfKind);
    }

    /// The wallet can pay more than it was asked for, most often by absorbing sub-dust change into
    /// the fee. Recording the target instead would understate the rate the next bump escalates
    /// from, and a target below what is already on the wire is a replacement Core rejects.
    #[test]
    fn effective_fee_rate_reflects_what_the_wallet_actually_paid() {
        let tx = commit_with(vec![p2wpkh_output(1_000)]);
        let vsize = tx.vsize() as u64;
        let fee = Amount::from_sat(vsize * 7 + 1);

        assert_eq!(
            effective_fee_rate(&tx, fee),
            FeeRate::from_sat_per_vb(8),
            "the rate rounds up so the recorded value is never under what was paid"
        );
    }

    #[test]
    fn effective_fee_rate_matches_the_target_when_the_wallet_paid_it_exactly() {
        let tx = commit_with(vec![p2wpkh_output(1_000)]);
        let fee = Amount::from_sat(tx.vsize() as u64 * 3);

        assert_eq!(effective_fee_rate(&tx, fee), FeeRate::from_sat_per_vb(3));
    }

    /// The mock wallet always reports a 0.01 BTC fee, which over this transaction is far more than
    /// the target asks for — the same overshoot Core produces when it absorbs sub-dust change.
    async fn wallet_replacement_with_ceiling(
        ceiling_sat_vb: u64,
    ) -> Result<TxAttempt, ReplacementError> {
        let original = commit_with(vec![p2wpkh_output(1_000)]);
        let client =
            TestBitcoinClient::new(1).with_wallet_process_psbt_result(true, Some(original.clone()));

        build_chunked_commit_replacement(
            &client,
            &original,
            L1TxId::from([0u8; 32]),
            FeeRate::from_sat_per_vb(4).expect("valid fee rate"),
            FeeRate::from_sat_per_vb(ceiling_sat_vb).expect("valid fee rate"),
            1,
            None,
        )
        .await
    }

    /// Regression: the target is capped by the ceiling, but the rate the wallet actually builds is
    /// not, and it is that transaction which reaches the wire and seeds the next bump's floors.
    #[tokio::test]
    async fn rejects_a_wallet_replacement_that_breaches_the_fee_rate_ceiling() {
        let error = wallet_replacement_with_ceiling(1_000)
            .await
            .expect_err("a replacement above the ceiling must not be adopted");

        assert!(matches!(error, ReplacementError::ExceedsMaxFeeRate { .. }));
        assert!(!error.is_retryable());
        assert_eq!(error.terminal_error(), TerminalError::AboveMaxFeeRate);
    }

    #[tokio::test]
    async fn accepts_a_wallet_replacement_that_stays_under_the_fee_rate_ceiling() {
        let attempt = wallet_replacement_with_ceiling(1_000_000)
            .await
            .expect("a replacement under the ceiling is adopted");

        assert_eq!(attempt.fee(), Amount::from_sat(1_000_000));
    }
}
