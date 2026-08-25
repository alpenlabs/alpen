use core::result::Result::Ok;
use std::{cmp::Reverse, slice};

use anyhow::anyhow;
use bitcoin::{
    absolute::LockTime,
    blockdata::script,
    hashes::Hash,
    key::UntweakedKeypair,
    secp256k1::{
        constants::SCHNORR_SIGNATURE_SIZE, schnorr::Signature, Message, XOnlyPublicKey, SECP256K1,
    },
    sighash::{Prevouts, SighashCache, TapSighashType, TaprootError},
    taproot::{
        ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootBuilderError,
        TaprootSpendInfo,
    },
    transaction::Version,
    Address, Amount, FeeRate, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
    Txid, Witness,
};
use bitcoind_async_client::{
    corepc_types::model::ListUnspentItem,
    error::ClientError,
    traits::{Reader, Signer, Wallet},
};
use rand::{rngs::OsRng, RngCore};
use strata_config::btcio::FeeBumpingConfig;
use strata_csm_types::L1Payload;
use strata_l1_envelope_fmt::{builder::EnvelopeScriptBuilder, errors::EnvelopeBuildError};
use strata_l1_txfmt::{self, MagicBytes, ParseConfig, TxFmtError};
use strata_primitives::buf::Buf32;
use thiserror::Error;

use super::context::WriterContext;
use crate::writer::fees::resolve_fee_rate;

pub(crate) const BITCOIN_DUST_LIMIT: u64 = 546;

/// Largest serialized ECDSA signature, including its sighash byte.
const MAX_ECDSA_SIGNATURE_SIZE: usize = 73;
const COMPRESSED_PUBLIC_KEY_SIZE: usize = 33;

/// Config for creating envelope transactions.
#[derive(Debug, Clone)]
pub struct EnvelopeConfig {
    /// Magic bytes for OP_RETURN tags in L1 transactions.
    pub magic_bytes: MagicBytes,
    /// Address to send change and reveal output to
    pub sequencer_address: Address,
    /// Amount to send to reveal address.
    ///
    /// Every production caller passes the Bitcoin dust limit, so this only varies in tests. It is
    /// not validated: a value below the dust limit would produce a non-standard reveal output.
    //
    // TODO(STR-3690): Make this and all other bitcoin related values to Amount
    pub reveal_amount: u64,
    /// Bitcoin network
    pub network: Network,
    /// Bitcoin fee rate.
    pub fee_rate: FeeRate,
    /// Fee-bumping policy used to derive reveal fee headroom.
    pub fee_bumping: FeeBumpingConfig,
    /// Sequencer public key for the taproot envelope script (SPS-51).
    ///
    /// Used as the `<pubkey>` in `<pubkey> CHECKSIG` of the envelope script.
    /// The ASM verifies the envelope was created by the authorized sequencer by
    /// checking this pubkey against the sequencer predicate.
    ///
    /// `None` when the caller generates ephemeral keypairs (chunked envelope path).
    pub envelope_pubkey: Option<XOnlyPublicKey>,
}

impl EnvelopeConfig {
    pub fn new(
        magic_bytes: MagicBytes,
        sequencer_address: Address,
        network: Network,
        fee_rate: FeeRate,
        reveal_amount: u64,
        fee_bumping: FeeBumpingConfig,
        envelope_pubkey: Option<XOnlyPublicKey>,
    ) -> Self {
        Self {
            magic_bytes,
            sequencer_address,
            reveal_amount,
            fee_rate,
            fee_bumping,
            network,
            envelope_pubkey,
        }
    }
}

// TODO(STR-2982): these might need to be in rollup params
#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("no payload provided")]
    EmptyPayload,

    #[error("insufficient funds for tx (need {0} sats, have {1} sats)")]
    NotEnoughUtxos(u64, u64),

    #[error("fee calculation overflowed")]
    FeeOverflow,

    #[error(
        "resolved fee rate {resolved_sat_vb} sat/vB exceeds broadcast ceiling {ceiling_sat_vb} sat/vB"
    )]
    ResolvedFeeRateAboveMax {
        resolved_sat_vb: u64,
        ceiling_sat_vb: u64,
    },

    #[error(
        "built transaction fee rate {built_sat_vb} sat/vB exceeds broadcast ceiling {ceiling_sat_vb} sat/vB"
    )]
    BuiltFeeRateAboveMax {
        built_sat_vb: u64,
        ceiling_sat_vb: u64,
    },

    #[error("Could not sign raw transaction: {0}")]
    SignRawTransaction(#[source] ClientError),

    #[error("envelope_pubkey is required for envelope transactions")]
    MissingEnvelopePubkey,

    #[error("chunked envelope sequencer/change address must not be P2TR")]
    P2trChangeAddressUnsupported,

    #[error("failed to fetch envelope prerequisites: {0}")]
    PrereqFetch(#[source] anyhow::Error),

    #[error("Error building taproot")]
    Taproot(#[from] TaprootBuilderError),

    #[error("sps tx fmt")]
    Tag(#[from] TxFmtError),

    #[error("envelope build error")]
    EnvelopeBuild(#[from] EnvelopeBuildError),

    #[error("failed to compute sighash")]
    Sighash(#[from] TaprootError),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

impl EnvelopeError {
    /// Reports whether transaction construction should wait for a fee rate below the guardrail.
    pub(crate) fn is_blocked_by_fee_guardrail(&self) -> bool {
        matches!(
            self,
            Self::ResolvedFeeRateAboveMax { .. } | Self::BuiltFeeRateAboveMax { .. }
        )
    }
}

/// Intermediate data held in the watcher's in-memory cache between envelope creation and reveal
/// broadcast.
///
/// Lost on restart, which safely resets the state machine to `Unsigned` because neither
/// transaction has been broadcast yet.
#[derive(Debug, Clone)]
pub struct EnvelopeData {
    /// The wallet-signed commit transaction (not yet broadcast).
    pub commit_tx: Transaction,
    /// The unsigned reveal transaction (no witness yet; needs external Schnorr sig).
    pub reveal_tx: Transaction,
    /// The taproot script-spend sighash that the external signer must sign.
    pub sighash: Buf32,
    /// The reveal script used in the taproot leaf.
    pub reveal_script: ScriptBuf,
    /// The taproot spend info for constructing the witness.
    pub taproot_spend_info: TaprootSpendInfo,
    /// The x-only public key committed to by the envelope reveal script.
    pub envelope_pubkey: XOnlyPublicKey,
    /// Fee rate the envelope transactions were built at.
    pub fee_rate: FeeRate,
    /// Absolute fee paid by the commit transaction.
    pub commit_fee: Amount,
    /// Absolute fee paid by the reveal transaction.
    pub reveal_fee: Amount,
}

impl EnvelopeData {
    #[expect(
        clippy::too_many_arguments,
        reason = "EnvelopeData is a plain construction bundle for transaction metadata"
    )]
    pub fn new(
        commit_tx: Transaction,
        reveal_tx: Transaction,
        sighash: Buf32,
        reveal_script: ScriptBuf,
        taproot_spend_info: TaprootSpendInfo,
        envelope_pubkey: XOnlyPublicKey,
        fee_rate: FeeRate,
        commit_fee: Amount,
        reveal_fee: Amount,
    ) -> Self {
        Self {
            commit_tx,
            reveal_tx,
            sighash,
            reveal_script,
            taproot_spend_info,
            envelope_pubkey,
            fee_rate,
            commit_fee,
            reveal_fee,
        }
    }
}

// This is hacky solution. As `btcio` has `transaction builder` that `tx-parser` depends on. But
// Btcio depends on `tx-parser`. So this file is behind a feature flag 'test-utils' and on dev
// dependencies on `tx-parser`, we include {btcio, feature="strata_test_utils"} , so cyclic
// dependency doesn't happen
pub(crate) async fn build_envelope_txs<R: Reader + Signer + Wallet>(
    payload: &L1Payload,
    ctx: &WriterContext<R>,
    envelope_pubkey: XOnlyPublicKey,
) -> Result<EnvelopeData, EnvelopeError> {
    let (network, utxos, fee_rate) = fetch_envelope_prereqs(ctx).await?;
    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
        ctx.config.fee_bumping,
        Some(envelope_pubkey),
    );
    let envelope = create_envelope_transactions(&env_config, payload, utxos)?;
    Ok(envelope)
}

/// Builds envelope transactions using a temporary keypair and signs both commit and reveal
/// in-process.
///
/// Used when no external signer is required.
pub(crate) async fn build_and_sign_envelope_txs<R: Reader + Signer + Wallet>(
    payload: &L1Payload,
    ctx: &WriterContext<R>,
) -> Result<EnvelopeData, EnvelopeError> {
    let (network, utxos, fee_rate) = fetch_envelope_prereqs(ctx).await?;
    let keypair = generate_key_pair()?;
    let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
        ctx.config.fee_bumping,
        Some(pubkey),
    );
    let mut envelope = create_envelope_transactions(&env_config, payload, utxos)?;

    let signed_commit = ctx
        .client
        .sign_raw_transaction_with_wallet(&envelope.commit_tx, None)
        .await
        .map_err(EnvelopeError::SignRawTransaction)?
        .tx;
    envelope.commit_tx = signed_commit;

    let output_to_reveal = envelope.commit_tx.output[0].clone();
    sign_reveal_transaction(
        &mut envelope.reveal_tx,
        &output_to_reveal,
        &envelope.reveal_script,
        &envelope.taproot_spend_info,
        &keypair,
    )?;

    Ok(envelope)
}

/// Fetches the shared prerequisites for building envelope transactions.
// TODO(STR-3411): make OL node resilient against the Bitcoin node not being available.
async fn fetch_envelope_prereqs<R: Reader + Signer + Wallet>(
    ctx: &WriterContext<R>,
) -> Result<(Network, Vec<ListUnspentItem>, FeeRate), EnvelopeError> {
    let network = ctx
        .client
        .network()
        .await
        .map_err(|error| EnvelopeError::PrereqFetch(error.into()))?;
    let utxos = ctx
        .client
        .list_unspent(None, None, None, None, None)
        .await
        .map_err(|error| EnvelopeError::PrereqFetch(error.into()))?
        .0;
    let fee_rate = resolve_fee_rate(ctx.client.as_ref(), ctx.config.as_ref())
        .await
        .map_err(EnvelopeError::PrereqFetch)?;
    ensure_initial_fee_rate_within_max(fee_rate, ctx.max_fee_rate)?;
    Ok((network, utxos, fee_rate))
}

/// Ensures an initial transaction is never built from an estimate above the broadcast ceiling.
pub(crate) fn ensure_initial_fee_rate_within_max(
    fee_rate: FeeRate,
    max_fee_rate: FeeRate,
) -> Result<(), EnvelopeError> {
    if fee_rate > max_fee_rate {
        return Err(EnvelopeError::ResolvedFeeRateAboveMax {
            resolved_sat_vb: fee_rate.to_sat_per_vb_ceil(),
            ceiling_sat_vb: max_fee_rate.to_sat_per_vb_ceil(),
        });
    }

    Ok(())
}

/// Returns a built transaction's effective fee rate, rounded up to whole sat/vB.
pub(crate) fn effective_fee_rate(tx: &Transaction, fee: Amount) -> Result<FeeRate, EnvelopeError> {
    let vsize = tx.vsize() as u64;
    if vsize == 0 {
        return Err(EnvelopeError::FeeOverflow);
    }

    FeeRate::from_sat_per_vb(fee.to_sat().div_ceil(vsize)).ok_or(EnvelopeError::FeeOverflow)
}

/// Ensures a built transaction's effective fee rate fits the broadcast ceiling.
pub(crate) fn ensure_built_fee_rate_within_max(
    tx: &Transaction,
    fee: Amount,
    max_fee_rate: FeeRate,
) -> Result<(), EnvelopeError> {
    let built_fee_rate = effective_fee_rate(tx, fee)?;
    if built_fee_rate > max_fee_rate {
        return Err(EnvelopeError::BuiltFeeRateAboveMax {
            built_sat_vb: built_fee_rate.to_sat_per_vb_ceil(),
            ceiling_sat_vb: max_fee_rate.to_sat_per_vb_ceil(),
        });
    }

    Ok(())
}

/// Builds unsigned envelope transactions (commit + reveal) and computes the sighash.
///
/// Returns an [`EnvelopeData`] containing the transactions and intermediate data
/// needed to attach the signature later via [`attach_reveal_signature`].
pub fn create_envelope_transactions(
    env_config: &EnvelopeConfig,
    payload: &L1Payload,
    utxos: Vec<ListUnspentItem>,
) -> Result<EnvelopeData, EnvelopeError> {
    let public_key = env_config
        .envelope_pubkey
        .ok_or(EnvelopeError::MissingEnvelopePubkey)?;

    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&public_key.serialize())?
        .add_envelopes(payload.data())?
        .build()?;

    let tag_script =
        ParseConfig::new(env_config.magic_bytes).encode_script_buf(&payload.tag().as_ref())?;

    // Create spend info for tapscript
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())?
        .finalize(SECP256K1, public_key)
        .map_err(|_| anyhow!("Could not build taproot spend info"))?;

    // Create reveal address
    let reveal_address = Address::p2tr(
        SECP256K1,
        public_key,
        taproot_spend_info.merkle_root(),
        env_config.network,
    );

    // Calculate commit value
    let commit_value = calculate_commit_output_value(
        &env_config.sequencer_address,
        env_config.reveal_amount,
        env_config.fee_rate,
        &reveal_script,
        &tag_script,
        &taproot_spend_info,
        &env_config.fee_bumping,
    )?;

    let reveal_vsize = single_reveal_vsize(
        &env_config.sequencer_address,
        env_config.reveal_amount,
        &reveal_script,
        &tag_script,
        &taproot_spend_info,
    );
    let reveal_vsize = u64::try_from(reveal_vsize).map_err(|_| EnvelopeError::FeeOverflow)?;
    let headroom = reveal_fee_headroom(env_config.fee_rate, reveal_vsize, &env_config.fee_bumping)?;
    let base_reveal_output_value = env_config
        .reveal_amount
        .checked_add(headroom)
        .ok_or(EnvelopeError::FeeOverflow)?;

    // Build commit tx
    let (commit_tx, consumed_utxos) = build_commit_transaction(
        utxos,
        reveal_address,
        env_config.sequencer_address.clone(),
        commit_value,
        env_config.fee_rate,
    )?;
    let commit_fee_sats = calculate_transaction_fee_from_utxos(&commit_tx, &consumed_utxos);

    let output_to_reveal = commit_tx.output[0].clone();
    let carried_change = output_to_reveal.value.to_sat().saturating_sub(commit_value);
    let reveal_output_value = base_reveal_output_value
        .checked_add(carried_change)
        .ok_or(EnvelopeError::FeeOverflow)?;

    // Build reveal tx
    let reveal_tx = build_reveal_transaction(
        commit_tx.clone(),
        env_config.sequencer_address.clone(),
        reveal_output_value,
        env_config.fee_rate,
        &reveal_script,
        tag_script,
        &taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or(anyhow!("Cannot create control block".to_string()))?,
    )?;
    let reveal_fee_sats = output_to_reveal.value.to_sat().saturating_sub(
        reveal_tx
            .output
            .iter()
            .map(|output| output.value.to_sat())
            .sum(),
    );

    // Compute sighash for the reveal tx
    let sighash = compute_reveal_sighash(&reveal_tx, &output_to_reveal, &reveal_script)?;

    Ok(EnvelopeData::new(
        commit_tx,
        reveal_tx,
        sighash,
        reveal_script,
        taproot_spend_info,
        public_key,
        env_config.fee_rate,
        Amount::from_sat(commit_fee_sats),
        Amount::from_sat(reveal_fee_sats),
    ))
}

fn calculate_transaction_fee_from_utxos(tx: &Transaction, utxos: &[ListUnspentItem]) -> u64 {
    let input_total = utxos.iter().map(|utxo| utxo.amount.to_sat()).sum::<u64>();
    let output_total = tx
        .output
        .iter()
        .map(|output| output.value.to_sat())
        .sum::<u64>();
    input_total.saturating_sub(output_total)
}

/// Computes the taproot script-spend sighash for the reveal transaction.
fn compute_reveal_sighash(
    reveal_tx: &Transaction,
    output_to_reveal: &TxOut,
    reveal_script: &ScriptBuf,
) -> Result<Buf32, EnvelopeError> {
    let mut sighash_cache = SighashCache::new(reveal_tx);
    let signature_hash = sighash_cache.taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(&[output_to_reveal]),
        TapLeafHash::from_script(reveal_script, LeafVersion::TapScript),
        TapSighashType::Default,
    )?;
    Ok(Buf32(*signature_hash.as_byte_array()))
}

pub(crate) fn get_size(
    inputs: &[TxIn],
    outputs: &[TxOut],
    script: Option<&ScriptBuf>,
    control_block: Option<&ControlBlock>,
) -> usize {
    let mut tx = Transaction {
        input: inputs.to_vec(),
        output: outputs.to_vec(),
        lock_time: LockTime::ZERO,
        version: Version(2),
    };

    for i in 0..tx.input.len() {
        // Safe: Creating a signature from a fixed-size array of correct length
        tx.input[i].witness.push(
            Signature::from_slice(&[0; SCHNORR_SIGNATURE_SIZE])
                .expect("valid signature size")
                .as_ref(),
        );
    }

    match (script, control_block) {
        (Some(sc), Some(cb)) if tx.input.len() == 1 => {
            tx.input[0].witness.push(sc);
            tx.input[0].witness.push(cb.serialize());
        }
        _ => {}
    }

    tx.vsize()
}

/// Returns whether signing this wallet output leaves the commit txid unchanged.
///
/// Reveals are built before the wallet signs the commit, so commit inputs must put their entire
/// satisfaction in the witness. The sequencer wallet normally produces P2WPKH outputs; P2TR key
/// spends are also safe and have a known witness shape.
fn is_supported_commit_utxo(utxo: &ListUnspentItem) -> bool {
    utxo.script_pubkey.is_p2wpkh() || utxo.script_pubkey.is_p2tr()
}

fn commit_inputs(utxos: &[ListUnspentItem]) -> Vec<TxIn> {
    utxos
        .iter()
        .map(|utxo| TxIn {
            previous_output: OutPoint {
                txid: utxo.txid,
                vout: utxo.vout,
            },
            script_sig: ScriptBuf::new(),
            witness: Witness::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        })
        .collect()
}

/// Estimates the signed commit vsize from the selected wallet outputs.
///
/// P2WPKH uses Bitcoin Core's conservative high-R witness shape. The wallet normally grinds a
/// low-R signature, so the final transaction can be up to one vbyte smaller; the signed
/// transaction's effective rate is checked against the hard broadcast ceiling before persistence.
pub(crate) fn signed_commit_vsize(utxos: &[ListUnspentItem], outputs: &[TxOut]) -> usize {
    let mut inputs = commit_inputs(utxos);
    for (input, utxo) in inputs.iter_mut().zip(utxos) {
        if utxo.script_pubkey.is_p2wpkh() {
            input.witness.push([0; MAX_ECDSA_SIGNATURE_SIZE]);
            input.witness.push([0; COMPRESSED_PUBLIC_KEY_SIZE]);
        } else if utxo.script_pubkey.is_p2tr() {
            input.witness.push([0; SCHNORR_SIGNATURE_SIZE]);
        } else {
            unreachable!("commit coin selection only returns supported native SegWit outputs");
        }
    }

    Transaction {
        input: inputs,
        output: outputs.to_vec(),
        lock_time: LockTime::ZERO,
        version: Version(2),
    }
    .vsize()
}

/// Choose utxos almost naively.
pub(crate) fn choose_utxos(
    utxos: &[ListUnspentItem],
    amount: u64,
) -> Result<(Vec<ListUnspentItem>, u64), EnvelopeError> {
    let mut bigger_utxos: Vec<&ListUnspentItem> = utxos
        .iter()
        .filter(|utxo| utxo.amount.to_sat() >= amount)
        .collect();
    let mut sum: u64 = 0;

    if !bigger_utxos.is_empty() {
        // sort vec by amount (small first)
        bigger_utxos.sort_by_key(|&x| x.amount);

        // single utxo will be enough
        // so return the transaction
        let utxo = bigger_utxos[0];
        sum += utxo.amount.to_sat();

        Ok((vec![utxo.clone()], sum))
    } else {
        let mut smaller_utxos: Vec<&ListUnspentItem> = utxos
            .iter()
            .filter(|utxo| utxo.amount.to_sat() < amount)
            .collect();

        // sort vec by amount (large first)
        smaller_utxos.sort_by_key(|x| Reverse(&x.amount));

        let mut chosen_utxos: Vec<ListUnspentItem> = vec![];

        for utxo in smaller_utxos {
            sum += utxo.amount.to_sat();
            chosen_utxos.push(utxo.clone());

            if sum >= amount {
                break;
            }
        }

        if sum < amount {
            return Err(EnvelopeError::NotEnoughUtxos(amount, sum));
        }

        Ok((chosen_utxos, sum))
    }
}

fn build_commit_transaction(
    utxos: Vec<ListUnspentItem>,
    recipient: Address,
    change_address: Address,
    output_value: u64,
    fee_rate: FeeRate,
) -> Result<(Transaction, Vec<ListUnspentItem>), EnvelopeError> {
    let (transaction, consumed_utxos, _) = fund_commit_transaction(
        utxos,
        vec![TxOut {
            script_pubkey: recipient.script_pubkey(),
            value: Amount::from_sat(output_value),
        }],
        change_address.script_pubkey(),
        0,
        fee_rate,
    )?;

    Ok((transaction, consumed_utxos))
}

/// Selects enough wallet outputs to fund `outputs` and the fee implied by their signed input size.
fn select_commit_utxos(
    utxos: &[ListUnspentItem],
    outputs: &[TxOut],
    base_output_total: u64,
    minimum_excess: u64,
    fee_rate: FeeRate,
) -> Result<(Vec<ListUnspentItem>, u64, u64), EnvelopeError> {
    let Some(first_utxo) = utxos.first() else {
        return Err(EnvelopeError::NotEnoughUtxos(base_output_total, 0));
    };
    let mut estimated_size = signed_commit_vsize(slice::from_ref(first_utxo), outputs);

    loop {
        let estimated_fee = fee_sats_for_vsize(estimated_size, fee_rate)?;
        let estimated_input_total = base_output_total
            .checked_add(estimated_fee)
            .and_then(|total| total.checked_add(minimum_excess))
            .ok_or(EnvelopeError::FeeOverflow)?;
        let (chosen_utxos, sum) = choose_utxos(utxos, estimated_input_total)?;
        let signed_size = signed_commit_vsize(&chosen_utxos, outputs);
        let fee = fee_sats_for_vsize(signed_size, fee_rate)?;
        let required = base_output_total
            .checked_add(fee)
            .and_then(|total| total.checked_add(minimum_excess))
            .ok_or(EnvelopeError::FeeOverflow)?;

        if sum >= required {
            return Ok((chosen_utxos, sum, fee));
        }

        estimated_size = signed_size;
    }
}

/// Funds fixed commit outputs at the requested rate and prefers bumpable change.
pub(crate) fn fund_commit_transaction(
    utxos: Vec<ListUnspentItem>,
    base_outputs: Vec<TxOut>,
    change_script: ScriptBuf,
    carry_output_index: usize,
    fee_rate: FeeRate,
) -> Result<(Transaction, Vec<ListUnspentItem>, Amount), EnvelopeError> {
    let base_output_total = base_outputs.iter().try_fold(0u64, |total, output| {
        total.checked_add(output.value.to_sat())
    });
    let base_output_total = base_output_total.ok_or(EnvelopeError::FeeOverflow)?;

    let utxos: Vec<ListUnspentItem> = utxos
        .iter()
        .filter(|utxo| {
            utxo.spendable
                && utxo.solvable
                && utxo.amount.to_sat() > BITCOIN_DUST_LIMIT
                && is_supported_commit_utxo(utxo)
        })
        .cloned()
        .collect();

    // Prefer a standalone change output even when doing so requires another wallet input. The fee
    // bumper can shrink that output without racing another writer for an unlocked input.
    let mut outputs_with_change = base_outputs.clone();
    outputs_with_change.push(TxOut {
        value: Amount::ZERO,
        script_pubkey: change_script,
    });
    match select_commit_utxos(
        &utxos,
        &outputs_with_change,
        base_output_total,
        BITCOIN_DUST_LIMIT,
        fee_rate,
    ) {
        Ok((chosen_utxos, sum, fee)) => {
            outputs_with_change
                .last_mut()
                .expect("change output was just appended")
                .value = Amount::from_sat(sum - base_output_total - fee);
            return Ok((
                Transaction {
                    lock_time: LockTime::ZERO,
                    version: Version(2),
                    input: commit_inputs(&chosen_utxos),
                    output: outputs_with_change,
                },
                chosen_utxos,
                Amount::from_sat(fee),
            ));
        }
        Err(EnvelopeError::NotEnoughUtxos(_, _)) => {}
        Err(error) => return Err(error),
    }

    // If the wallet cannot fund spendable change, carry every non-fee satoshi through a reveal
    // output. This preserves the requested fee and avoids accidentally burning the remainder.
    let (chosen_utxos, sum, fee) =
        select_commit_utxos(&utxos, &base_outputs, base_output_total, 0, fee_rate)?;
    let carried_value = sum
        .checked_sub(base_output_total)
        .and_then(|value| value.checked_sub(fee))
        .expect("coin selection funded the fixed outputs and fee");
    let mut outputs = base_outputs;
    let carry_output = outputs
        .get_mut(carry_output_index)
        .expect("carry output index must reference a fixed output");
    carry_output.value = carry_output
        .value
        .checked_add(Amount::from_sat(carried_value))
        .ok_or(EnvelopeError::FeeOverflow)?;

    Ok((
        Transaction {
            lock_time: LockTime::ZERO,
            version: Version(2),
            input: commit_inputs(&chosen_utxos),
            output: outputs,
        },
        chosen_utxos,
        Amount::from_sat(fee),
    ))
}

fn default_txin() -> Vec<TxIn> {
    vec![TxIn {
        previous_output: OutPoint {
            txid: Txid::all_zeros(),
            vout: 0,
        },
        script_sig: script::Builder::new().into_script(),
        witness: Witness::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
    }]
}

pub fn build_reveal_transaction(
    input_transaction: Transaction,
    recipient: Address,
    output_value: u64,
    fee_rate: FeeRate,
    reveal_script: &ScriptBuf,
    tag_script: ScriptBuf,
    control_block: &ControlBlock,
) -> Result<Transaction, EnvelopeError> {
    let outputs: Vec<TxOut> = vec![
        // The first output should be SPS-50 tagged
        TxOut {
            value: Amount::from_sat(0),
            script_pubkey: tag_script,
        },
        TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: recipient.script_pubkey(),
        },
    ];

    let v_out_for_reveal = 0u32;
    let input_utxo = input_transaction.output[v_out_for_reveal as usize].clone();
    let txn_id = input_transaction.compute_txid();

    let inputs = vec![TxIn {
        previous_output: OutPoint {
            txid: txn_id,
            vout: v_out_for_reveal,
        },
        script_sig: script::Builder::new().into_script(),
        witness: Witness::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
    }];
    let size = get_size(&inputs, &outputs, Some(reveal_script), Some(control_block));
    let fee = fee_sats_for_vsize(size, fee_rate)?;
    let input_required = Amount::from_sat(
        output_value
            .checked_add(fee)
            .ok_or(EnvelopeError::FeeOverflow)?,
    );
    if input_utxo.value < Amount::from_sat(BITCOIN_DUST_LIMIT) || input_utxo.value < input_required
    {
        return Err(EnvelopeError::NotEnoughUtxos(
            input_required.to_sat(),
            input_utxo.value.to_sat(),
        ));
    }
    let tx = Transaction {
        lock_time: LockTime::ZERO,
        version: Version(2),
        input: inputs,
        output: outputs,
    };

    Ok(tx)
}

pub(crate) fn calculate_commit_output_value(
    recipient: &Address,
    reveal_value: u64,
    fee_rate: FeeRate,
    reveal_script: &script::ScriptBuf,
    tag_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
    fee_bumping: &FeeBumpingConfig,
) -> Result<u64, EnvelopeError> {
    let reveal_vsize = single_reveal_vsize(
        recipient,
        reveal_value,
        reveal_script,
        tag_script,
        taproot_spend_info,
    );
    let reveal_vsize = u64::try_from(reveal_vsize).map_err(|_| EnvelopeError::FeeOverflow)?;
    let fee = fee_rate
        .fee_vb(reveal_vsize)
        .ok_or(EnvelopeError::FeeOverflow)?
        .to_sat();
    let headroom = reveal_fee_headroom(fee_rate, reveal_vsize, fee_bumping)?;
    reveal_value
        .checked_add(headroom)
        .and_then(|value| value.checked_add(fee))
        .ok_or(EnvelopeError::FeeOverflow)
}

fn single_reveal_vsize(
    recipient: &Address,
    reveal_value: u64,
    reveal_script: &script::ScriptBuf,
    tag_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
) -> usize {
    get_size(
        &default_txin(),
        &[
            TxOut {
                script_pubkey: tag_script.clone(),
                value: Amount::from_sat(0),
            },
            TxOut {
                script_pubkey: recipient.script_pubkey(),
                value: Amount::from_sat(reveal_value),
            },
        ],
        Some(reveal_script),
        Some(
            &taproot_spend_info
                .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
                .expect("Cannot create control block"),
        ),
    )
}

/// Derives the absolute fee headroom needed to fund the configured reveal replacement schedule.
pub(crate) fn reveal_fee_headroom(
    build_rate: FeeRate,
    reveal_vsize: u64,
    config: &FeeBumpingConfig,
) -> Result<u64, EnvelopeError> {
    let mut rate = build_rate.to_sat_per_vb_ceil();
    let max_rate = config.max_fee_rate_sat_vb.get();

    for _ in 0..config.max_attempts.get().saturating_sub(1) {
        let additive = rate.saturating_add(config.min_fee_rate_delta_sat_vb.get());
        let multiplicative = u128::from(rate)
            .saturating_mul(u128::from(config.multiplier_bps))
            .div_ceil(10_000);
        let multiplicative = u64::try_from(multiplicative).unwrap_or(u64::MAX);
        rate = additive.max(multiplicative).min(max_rate);
        if rate >= max_rate {
            break;
        }
    }

    let top_fee = FeeRate::from_sat_per_vb(rate)
        .and_then(|fee_rate| fee_rate.fee_vb(reveal_vsize))
        .ok_or(EnvelopeError::FeeOverflow)?;
    let base_fee = build_rate
        .fee_vb(reveal_vsize)
        .ok_or(EnvelopeError::FeeOverflow)?;

    Ok(top_fee
        .to_sat()
        .saturating_sub(base_fee.to_sat())
        .min(config.max_reveal_fee_headroom().to_sat()))
}

pub(crate) fn fee_sats_for_vsize(vsize: usize, fee_rate: FeeRate) -> Result<u64, EnvelopeError> {
    let vsize = u64::try_from(vsize).map_err(|_| EnvelopeError::FeeOverflow)?;
    fee_rate
        .fee_vb(vsize)
        .map(|fee| fee.to_sat())
        .ok_or(EnvelopeError::FeeOverflow)
}

/// Generates a random keypair for envelope construction.
///
/// Used by the unchecked single-payload envelope path when no external
/// reveal signer is configured. The normal signed single-payload path uses
/// `envelope_pubkey` and attaches the external signer's signature later.
pub fn generate_key_pair() -> Result<UntweakedKeypair, anyhow::Error> {
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    Ok(UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes)?)
}

/// Signs and attaches a taproot script-spend witness to the reveal transaction.
///
/// Used by in-process signing paths. The caller owns which keypair is valid
/// for the reveal script.
pub(crate) fn sign_reveal_transaction(
    reveal_tx: &mut Transaction,
    output_to_reveal: &TxOut,
    reveal_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
    key_pair: &UntweakedKeypair,
) -> Result<(), anyhow::Error> {
    let sighash = compute_reveal_sighash(reveal_tx, output_to_reveal, reveal_script)?;

    let mut randbytes = [0; 32];
    OsRng.fill_bytes(&mut randbytes);
    let sig = SECP256K1.sign_schnorr_with_aux_rand(
        &Message::from_digest_slice(&sighash.0)?,
        key_pair,
        &randbytes,
    );

    attach_reveal_signature(reveal_tx, reveal_script, taproot_spend_info, sig.as_ref())
}

/// Attaches a pre-computed Schnorr signature to the reveal transaction witness.
///
/// The signature must be a valid BIP-340 Schnorr signature over the sighash
/// returned by [`create_envelope_transactions`].
pub fn attach_reveal_signature(
    reveal_tx: &mut Transaction,
    reveal_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
    signature: &[u8; 64],
) -> Result<(), anyhow::Error> {
    let sig =
        Signature::from_slice(signature).map_err(|e| anyhow!("invalid schnorr signature: {e}"))?;

    let witness = &mut reveal_tx.input[0].witness;
    witness.push(sig.as_ref());
    witness.push(reveal_script);
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or(anyhow!("Could not create control block"))?
            .serialize(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        num::{NonZeroU32, NonZeroU64},
        sync::Arc,
    };

    use bitcoin::{
        absolute::LockTime,
        script,
        secp256k1::{constants::SCHNORR_SIGNATURE_SIZE, Secp256k1, SecretKey},
        taproot::ControlBlock,
        transaction::Version,
        Address, Network, OutPoint, PubkeyHash, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use bitcoind_async_client::corepc_types::model::ListUnspentItem;
    use strata_l1_txfmt::{MagicBytes, TagData, TagDataRef};

    use super::*;
    use crate::{
        test_utils::{test_context::get_writer_context, TestBitcoinClient},
        writer::builder::EnvelopeError,
    };

    fn get_mock_data() -> (
        Arc<WriterContext<TestBitcoinClient>>,
        Vec<u8>,
        Vec<u8>,
        Vec<ListUnspentItem>,
    ) {
        let ctx = get_writer_context();
        let body = vec![100; 1000];
        let signature = vec![100; 64];
        let address = ctx.sequencer_address.clone();

        let utxos = vec![
            ListUnspentItem {
                txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.as_unchecked().clone(),
                script_pubkey: address.script_pubkey(),
                amount: Amount::from_btc(100.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: "".to_string(),
                safe: true,
                redeem_script: None,
                descriptor: None,
                parent_descriptors: None,
            },
            ListUnspentItem {
                txid: "44990141674ff56ed6fee38879e497b2a726cddefd5e4d9b7bf1c4e561de4347"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.as_unchecked().clone(),
                script_pubkey: address.script_pubkey(),
                amount: Amount::from_btc(50.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: "".to_string(),
                safe: true,
                redeem_script: None,
                descriptor: None,
                parent_descriptors: None,
            },
            ListUnspentItem {
                txid: "4dbe3c10ee0d6bf16f9417c68b81e963b5bccef3924bbcb0885c9ea841912325"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.as_unchecked().clone(),
                script_pubkey: address.script_pubkey(),
                amount: Amount::from_btc(10.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: "".to_string(),
                safe: true,
                redeem_script: None,
                descriptor: None,
                parent_descriptors: None,
            },
        ];

        (ctx, body, signature, utxos)
    }

    fn test_payload() -> L1Payload {
        let tag = TagData::new(1, 1, vec![]).unwrap();
        L1Payload::new(vec![vec![0u8; 150]], tag).unwrap()
    }

    fn test_envelope_pubkey() -> XOnlyPublicKey {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let (pubkey, _) = sk.x_only_public_key(&secp);
        pubkey
    }

    fn test_envelope_config(
        ctx: &WriterContext<TestBitcoinClient>,
        envelope_pubkey: Option<XOnlyPublicKey>,
    ) -> EnvelopeConfig {
        EnvelopeConfig::new(
            MagicBytes::new(*b"ALPN"),
            ctx.sequencer_address.clone(),
            Network::Regtest,
            FeeRate::from_sat_per_vb_u32(1_000),
            546,
            FeeBumpingConfig::default(),
            envelope_pubkey,
        )
    }

    #[test]
    fn choose_utxos() {
        let (_, _, _, utxos) = get_mock_data();

        let (chosen_utxos, sum) = super::choose_utxos(&utxos, 500_000_000).unwrap();

        assert_eq!(sum, 1_000_000_000);
        assert_eq!(chosen_utxos.len(), 1);
        assert_eq!(chosen_utxos[0], utxos[2]);

        let (chosen_utxos, sum) = super::choose_utxos(&utxos, 1_000_000_000).unwrap();

        assert_eq!(sum, 1_000_000_000);
        assert_eq!(chosen_utxos.len(), 1);
        assert_eq!(chosen_utxos[0], utxos[2]);

        let (chosen_utxos, sum) = super::choose_utxos(&utxos, 2_000_000_000).unwrap();

        assert_eq!(sum, 5_000_000_000);
        assert_eq!(chosen_utxos.len(), 1);
        assert_eq!(chosen_utxos[0], utxos[1]);

        let (chosen_utxos, sum) = super::choose_utxos(&utxos, 15_500_000_000).unwrap();

        assert_eq!(sum, 16_000_000_000);
        assert_eq!(chosen_utxos.len(), 3);
        assert_eq!(chosen_utxos[0], utxos[0]);
        assert_eq!(chosen_utxos[1], utxos[1]);
        assert_eq!(chosen_utxos[2], utxos[2]);

        let res = super::choose_utxos(&utxos, 50_000_000_000);

        assert!(matches!(
            res,
            Err(EnvelopeError::NotEnoughUtxos(50_000_000_000, _))
        ));
    }

    #[test]
    fn initial_fee_rate_above_broadcast_ceiling_is_blocked() {
        let ceiling = FeeRate::from_sat_per_vb(100).unwrap();

        assert!(ensure_initial_fee_rate_within_max(ceiling, ceiling).is_ok());
        assert!(matches!(
            ensure_initial_fee_rate_within_max(FeeRate::from_sat_per_vb(101).unwrap(), ceiling),
            Err(EnvelopeError::ResolvedFeeRateAboveMax {
                resolved_sat_vb: 101,
                ceiling_sat_vb: 100,
            })
        ));
    }

    #[test]
    fn built_fee_rate_above_broadcast_ceiling_is_blocked() {
        let (_, _, _, utxos) = get_mock_data();
        let tx = get_txn_from_utxo(&utxos[0], &get_writer_context().sequencer_address);
        let ceiling = FeeRate::from_sat_per_vb(100).unwrap();
        let fee_at_ceiling = ceiling.fee_vb(tx.vsize() as u64).unwrap();

        assert!(ensure_built_fee_rate_within_max(&tx, fee_at_ceiling, ceiling).is_ok());
        assert!(matches!(
            ensure_built_fee_rate_within_max(
                &tx,
                fee_at_ceiling + Amount::from_sat(tx.vsize() as u64),
                ceiling,
            ),
            Err(EnvelopeError::BuiltFeeRateAboveMax {
                built_sat_vb: 101,
                ceiling_sat_vb: 100,
            })
        ));
    }

    #[test]
    fn commit_returns_sub_dust_change_through_reveal_output() {
        let (ctx, _, _, mut utxos) = get_mock_data();
        let output_value = 100_000;
        let returned_dust = 100;
        let fee_rate = FeeRate::from_sat_per_vb(10).unwrap();
        let recipient = ctx.sequencer_address.clone();
        utxos.truncate(2);
        let initial_size = signed_commit_vsize(
            &utxos,
            &[TxOut {
                value: Amount::from_sat(output_value),
                script_pubkey: recipient.script_pubkey(),
            }],
        );
        let requested_fee = fee_sats_for_vsize(initial_size, fee_rate).unwrap();
        let input_total = output_value + requested_fee + returned_dust;
        utxos[0].amount = Amount::from_sat(input_total / 2);
        utxos[1].amount = Amount::from_sat(input_total - input_total / 2);

        let (tx, consumed) =
            build_commit_transaction(utxos, recipient.clone(), recipient, output_value, fee_rate)
                .unwrap();
        let actual_fee = calculate_transaction_fee_from_utxos(&tx, &consumed);

        assert_eq!(tx.input.len(), 2);
        assert_eq!(tx.output.len(), 1);
        assert_eq!(tx.output[0].value.to_sat(), output_value + returned_dust);
        assert_eq!(actual_fee, requested_fee);
    }

    #[test]
    fn commit_selects_another_utxo_to_create_bumpable_change() {
        let (ctx, _, _, mut utxos) = get_mock_data();
        let output_value = 100_000;
        let fee_rate = FeeRate::from_sat_per_vb(10).unwrap();
        let recipient = ctx.sequencer_address.clone();
        utxos.truncate(2);
        let no_change_size = signed_commit_vsize(
            &utxos[..1],
            &[TxOut {
                value: Amount::from_sat(output_value),
                script_pubkey: recipient.script_pubkey(),
            }],
        );
        let no_change_fee = fee_sats_for_vsize(no_change_size, fee_rate).unwrap();
        utxos[0].amount = Amount::from_sat(output_value + no_change_fee + 100);
        utxos[1].amount = Amount::from_sat(10_000);

        let (tx, consumed) =
            build_commit_transaction(utxos, recipient.clone(), recipient, output_value, fee_rate)
                .unwrap();
        let actual_fee = calculate_transaction_fee_from_utxos(&tx, &consumed);
        let priced_vsize = signed_commit_vsize(&consumed, &tx.output);

        assert_eq!(tx.input.len(), 2);
        assert_eq!(tx.output.len(), 2);
        assert!(tx.output[1].value.to_sat() >= BITCOIN_DUST_LIMIT);
        assert_eq!(
            actual_fee,
            fee_sats_for_vsize(priced_vsize, fee_rate).unwrap()
        );
    }

    #[test]
    fn commit_pricing_includes_p2wpkh_signature_and_pubkey() {
        let (ctx, _, _, utxos) = get_mock_data();
        let outputs = [TxOut {
            value: Amount::from_sat(100_000),
            script_pubkey: ctx.sequencer_address.script_pubkey(),
        }];
        let unsigned_inputs = commit_inputs(&utxos[..1]);
        let schnorr_only_vsize = get_size(&unsigned_inputs, &outputs, None, None);
        let p2wpkh_vsize = signed_commit_vsize(&utxos[..1], &outputs);

        assert!(p2wpkh_vsize > schnorr_only_vsize);
    }

    #[test]
    fn forwarded_commit_change_does_not_reduce_recorded_reveal_fee() {
        let (ctx, _, _, mut utxos) = get_mock_data();
        let env_config = test_envelope_config(&ctx, Some(test_envelope_pubkey()));
        let payload = test_payload();
        let baseline = create_envelope_transactions(&env_config, &payload, utxos.clone()).unwrap();
        let commit_value = baseline.commit_tx.output[0].value.to_sat();
        utxos.truncate(2);
        let commit_size = signed_commit_vsize(
            &utxos,
            &[TxOut {
                value: Amount::from_sat(commit_value),
                script_pubkey: baseline.commit_tx.output[0].script_pubkey.clone(),
            }],
        );
        let commit_fee = fee_sats_for_vsize(commit_size, env_config.fee_rate).unwrap();
        let returned_dust = 100;
        let input_total = commit_value + commit_fee + returned_dust;
        utxos[0].amount = Amount::from_sat(input_total / 2);
        utxos[1].amount = Amount::from_sat(input_total - input_total / 2);

        let envelope = create_envelope_transactions(&env_config, &payload, utxos).unwrap();
        let reveal_output_total: Amount = envelope
            .reveal_tx
            .output
            .iter()
            .map(|output| output.value)
            .sum();
        let actual_reveal_fee = envelope.commit_tx.output[0].value - reveal_output_total;

        assert_eq!(envelope.commit_tx.input.len(), 2);
        assert_eq!(envelope.commit_fee, Amount::from_sat(commit_fee));
        assert_eq!(envelope.reveal_fee, actual_reveal_fee);
        assert_eq!(
            envelope.reveal_tx.output[1].value.to_sat(),
            baseline.reveal_tx.output[1].value.to_sat() + returned_dust
        );
    }

    fn get_txn_from_utxo(utxo: &ListUnspentItem, _address: &Address) -> Transaction {
        let inputs = vec![TxIn {
            previous_output: OutPoint {
                txid: utxo.txid,
                vout: utxo.vout,
            },
            script_sig: script::Builder::new().into_script(),
            witness: Witness::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
        }];

        let outputs = vec![TxOut {
            value: utxo.amount,
            script_pubkey: utxo.address.clone().assume_checked().script_pubkey(),
        }];

        Transaction {
            lock_time: LockTime::ZERO,
            version: Version(2),
            input: inputs,
            output: outputs,
        }
    }

    fn assert_all_inputs_opt_into_rbf(tx: &Transaction) {
        assert!(
            tx.input
                .iter()
                .all(|input| input.sequence == Sequence::ENABLE_RBF_NO_LOCKTIME),
            "all writer-built transaction inputs must opt into RBF"
        );
    }

    #[test]
    fn test_build_reveal_transaction() {
        let (ctx, _, _, utxos) = get_mock_data();

        let utxo = utxos.first().unwrap();
        let _reveal_script = ScriptBuf::from_hex("62a58f2674fd840b6144bea2e63ebd35c16d7fd40252a2f28b2a01a648df356343e47976d7906a0e688bf5e134b6fd21bd365c016b57b1ace85cf30bf1206e27").unwrap();

        let td = TagDataRef::new(1, 1, &[]).unwrap();
        let tag_script = ParseConfig::new((*b"ALPN").into())
            .encode_script_buf(&td)
            .unwrap();

        let control_block = ControlBlock::decode(&[
            193, 165, 246, 250, 6, 222, 28, 9, 130, 28, 217, 67, 171, 11, 229, 62, 48, 206, 219,
            111, 155, 208, 6, 7, 119, 63, 146, 90, 227, 254, 231, 232, 249,
        ])
        .unwrap(); // should be 33 bytes

        let fee_rate = FeeRate::from_sat_per_vb_u32(8);
        let reveal_vsize = get_size(
            &default_txin(),
            &[
                TxOut {
                    value: Amount::ZERO,
                    script_pubkey: tag_script.clone(),
                },
                TxOut {
                    value: Amount::from_sat(ctx.config.reveal_amount),
                    script_pubkey: ctx.sequencer_address.script_pubkey(),
                },
            ],
            Some(&_reveal_script),
            Some(&control_block),
        );
        let headroom = reveal_fee_headroom(
            fee_rate,
            u64::try_from(reveal_vsize).unwrap(),
            &ctx.config.fee_bumping,
        )
        .unwrap();
        let reveal_output_value = ctx.config.reveal_amount.checked_add(headroom).unwrap();
        let inp_txn = get_txn_from_utxo(utxo, &ctx.sequencer_address);
        let mut tx = super::build_reveal_transaction(
            inp_txn,
            ctx.sequencer_address.clone(),
            reveal_output_value,
            fee_rate,
            &_reveal_script,
            tag_script.clone(),
            &control_block,
        )
        .unwrap();

        tx.input[0].witness.push([0; SCHNORR_SIGNATURE_SIZE]);
        tx.input[0].witness.push(_reveal_script.clone());
        tx.input[0].witness.push(control_block.serialize());

        assert_eq!(tx.input.len(), 1);
        assert_all_inputs_opt_into_rbf(&tx);
        assert_eq!(tx.input[0].previous_output.vout, utxo.vout);

        assert_eq!(tx.output.len(), 2);
        assert_eq!(
            tx.output[1].value.to_sat(),
            ctx.config.reveal_amount + headroom
        );
        assert_eq!(
            tx.output[1].script_pubkey,
            ctx.sequencer_address.script_pubkey()
        );

        // Test not enough utxos
        let utxo = utxos.get(2).unwrap();
        let inp_txn = get_txn_from_utxo(utxo, &ctx.sequencer_address);
        let inp_required = 5000000000;
        let tx = super::build_reveal_transaction(
            inp_txn,
            ctx.sequencer_address.clone(),
            inp_required,
            FeeRate::from_sat_per_vb_u32(750),
            &_reveal_script,
            tag_script,
            &control_block,
        );

        assert!(tx.is_err());
        assert!(matches!(tx, Err(EnvelopeError::NotEnoughUtxos(_, _))));
    }

    #[test]
    fn test_create_envelope_transactions_requires_envelope_pubkey() {
        let (ctx, _, _, utxos) = get_mock_data();
        let env_config = test_envelope_config(&ctx, None);

        let res = super::create_envelope_transactions(&env_config, &test_payload(), utxos);

        assert!(matches!(res, Err(EnvelopeError::MissingEnvelopePubkey)));
    }

    #[test]
    fn test_build_commit_transaction_filters_unusable_utxos() {
        let (ctx, _, _, utxos) = get_mock_data();
        let mut unspendable = utxos[0].clone();
        unspendable.spendable = false;
        let mut unsolvable = utxos[1].clone();
        unsolvable.solvable = false;
        let mut legacy = utxos[2].clone();
        legacy.script_pubkey = ScriptBuf::new_p2pkh(&PubkeyHash::all_zeros());
        legacy.amount = Amount::from_btc(100.0).unwrap();
        let viable = utxos[2].clone();

        let (tx, consumed) = super::build_commit_transaction(
            vec![unspendable, unsolvable, legacy, viable.clone()],
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            500_000_000,
            FeeRate::from_sat_per_vb_u32(1),
        )
        .unwrap();

        assert_eq!(consumed, vec![viable.clone()]);
        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].previous_output.txid, viable.txid);
    }

    #[test]
    fn test_build_commit_transaction_rejects_insufficient_filtered_utxos() {
        let (ctx, _, _, utxos) = get_mock_data();
        let mut dust = utxos[2].clone();
        dust.amount = Amount::from_sat(BITCOIN_DUST_LIMIT);

        let res = super::build_commit_transaction(
            vec![dust],
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            500_000_000,
            FeeRate::from_sat_per_vb_u32(1),
        );

        assert!(matches!(res, Err(EnvelopeError::NotEnoughUtxos(_, 0))));
    }

    #[test]
    fn test_build_reveal_transaction_rejects_dust_input() {
        let (ctx, _, _, utxos) = get_mock_data();
        let mut dust = utxos[2].clone();
        dust.amount = Amount::from_sat(BITCOIN_DUST_LIMIT - 1);
        let input_tx = get_txn_from_utxo(&dust, &ctx.sequencer_address);
        let reveal_script = ScriptBuf::from_hex("62a58f2674fd840b6144bea2e63ebd35c16d7fd40252a2f28b2a01a648df356343e47976d7906a0e688bf5e134b6fd21bd365c016b57b1ace85cf30bf1206e27").unwrap();
        let tag = TagDataRef::new(1, 1, &[]).unwrap();
        let tag_script = ParseConfig::new((*b"ALPN").into())
            .encode_script_buf(&tag)
            .unwrap();
        let control_block = ControlBlock::decode(&[
            193, 165, 246, 250, 6, 222, 28, 9, 130, 28, 217, 67, 171, 11, 229, 62, 48, 206, 219,
            111, 155, 208, 6, 7, 119, 63, 146, 90, 227, 254, 231, 232, 249,
        ])
        .unwrap();

        let res = super::build_reveal_transaction(
            input_tx,
            ctx.sequencer_address.clone(),
            ctx.config.reveal_amount,
            FeeRate::from_sat_per_vb_u32(1),
            &reveal_script,
            tag_script,
            &control_block,
        );

        assert!(matches!(res, Err(EnvelopeError::NotEnoughUtxos(_, _))));
    }

    #[test]
    fn test_create_envelope_transactions() {
        let (ctx, _, _, utxos) = get_mock_data();

        let payload = test_payload();
        let env_config = test_envelope_config(&ctx, Some(test_envelope_pubkey()));
        let unsigned =
            super::create_envelope_transactions(&env_config, &payload, utxos.to_vec()).unwrap();

        // check outputs
        assert_eq!(
            unsigned.commit_tx.output.len(),
            2,
            "commit tx should have 2 outputs"
        );

        assert_eq!(
            unsigned.reveal_tx.output.len(),
            2,
            "reveal tx should have 2 outputs"
        );

        assert_eq!(
            unsigned.commit_tx.input[0].previous_output.txid, utxos[2].txid,
            "utxo should be chosen correctly"
        );
        assert_eq!(
            unsigned.commit_tx.input[0].previous_output.vout, utxos[2].vout,
            "utxo should be chosen correctly"
        );
        assert_all_inputs_opt_into_rbf(&unsigned.commit_tx);

        assert_eq!(
            unsigned.reveal_tx.input[0].previous_output.txid,
            unsigned.commit_tx.compute_txid(),
            "reveal should use commit as input"
        );
        assert_eq!(
            unsigned.reveal_tx.input[0].previous_output.vout, 0,
            "reveal should use commit as input"
        );
        assert_all_inputs_opt_into_rbf(&unsigned.reveal_tx);

        assert_eq!(
            unsigned.reveal_tx.output[1].script_pubkey,
            ctx.sequencer_address.script_pubkey(),
            "reveal should pay to the correct address"
        );

        // Sighash should be non-zero
        assert_ne!(unsigned.sighash, Buf32::zero());
    }

    #[test]
    fn single_envelope_funds_headroom_without_burning_it() {
        let (ctx, _, _, utxos) = get_mock_data();
        let env_config = EnvelopeConfig {
            fee_rate: FeeRate::from_sat_per_vb(1).unwrap(),
            ..test_envelope_config(&ctx, Some(test_envelope_pubkey()))
        };
        let envelope = create_envelope_transactions(&env_config, &test_payload(), utxos).unwrap();
        let tag_script = ParseConfig::new(env_config.magic_bytes)
            .encode_script_buf(&test_payload().tag().as_ref())
            .unwrap();
        let reveal_vsize = single_reveal_vsize(
            &env_config.sequencer_address,
            env_config.reveal_amount,
            &envelope.reveal_script,
            &tag_script,
            &envelope.taproot_spend_info,
        );
        let reveal_vsize = u64::try_from(reveal_vsize).unwrap();
        let base_fee = env_config.fee_rate.fee_vb(reveal_vsize).unwrap();
        let headroom =
            reveal_fee_headroom(env_config.fee_rate, reveal_vsize, &env_config.fee_bumping)
                .unwrap();

        assert!(headroom > 0);
        assert_eq!(
            envelope.commit_tx.output[0].value.to_sat(),
            env_config.reveal_amount + base_fee.to_sat() + headroom
        );
        assert_eq!(
            envelope.reveal_tx.output[1].value.to_sat(),
            env_config.reveal_amount + headroom
        );
        assert_eq!(envelope.reveal_fee, base_fee);
    }

    #[test]
    fn zero_derived_headroom_preserves_legacy_single_envelope_values() {
        let (ctx, _, _, utxos) = get_mock_data();
        let env_config = EnvelopeConfig {
            fee_rate: FeeRate::from_sat_per_vb(1).unwrap(),
            fee_bumping: FeeBumpingConfig {
                max_attempts: NonZeroU32::new(1).unwrap(),
                ..FeeBumpingConfig::default()
            },
            ..test_envelope_config(&ctx, Some(test_envelope_pubkey()))
        };
        let envelope = create_envelope_transactions(&env_config, &test_payload(), utxos).unwrap();
        let base_fee = envelope.reveal_fee.to_sat();

        assert_eq!(
            envelope.commit_tx.output[0].value.to_sat(),
            env_config.reveal_amount + base_fee
        );
        assert_eq!(
            envelope.reveal_tx.output[1].value.to_sat(),
            env_config.reveal_amount
        );
    }

    #[test]
    fn reveal_headroom_cap_binds_in_isolation() {
        let fee_bumping = FeeBumpingConfig {
            max_reveal_fee_headroom_sats: NonZeroU64::new(7).unwrap(),
            ..FeeBumpingConfig::default()
        };

        assert_eq!(
            reveal_fee_headroom(FeeRate::from_sat_per_vb(1).unwrap(), 200, &fee_bumping,).unwrap(),
            7
        );
    }

    #[test]
    fn build_rate_at_or_above_max_rate_has_no_headroom() {
        let fee_bumping = FeeBumpingConfig::default();

        for build_rate in [1_000, 1_001] {
            assert_eq!(
                reveal_fee_headroom(
                    FeeRate::from_sat_per_vb(build_rate).unwrap(),
                    200,
                    &fee_bumping,
                )
                .unwrap(),
                0
            );
        }
    }

    #[test]
    fn test_attach_reveal_signature_populates_witness() {
        let (ctx, _, _, utxos) = get_mock_data();
        let payload = test_payload();
        let env_config = test_envelope_config(&ctx, Some(test_envelope_pubkey()));
        let mut unsigned =
            super::create_envelope_transactions(&env_config, &payload, utxos.to_vec()).unwrap();

        super::attach_reveal_signature(
            &mut unsigned.reveal_tx,
            &unsigned.reveal_script,
            &unsigned.taproot_spend_info,
            &[0x11; SCHNORR_SIGNATURE_SIZE],
        )
        .unwrap();

        let witness = &unsigned.reveal_tx.input[0].witness;
        assert_eq!(witness.len(), 3);
        assert_eq!(witness.iter().next().unwrap().len(), SCHNORR_SIGNATURE_SIZE);
    }
}
