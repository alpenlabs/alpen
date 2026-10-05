use core::result::Result::Ok;
use std::cmp::Reverse;

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
use strata_csm_types::L1Payload;
use strata_l1_envelope_fmt::{EnvelopeBuildError, EnvelopeScriptBuilder};
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
    /// NOTE: must be higher than the dust limit.
    //
    // TODO(STR-3690): Make this and all other bitcoin related values to Amount
    pub reveal_amount: u64,
    /// Bitcoin network
    pub network: Network,
    /// Bitcoin fee rate.
    pub fee_rate: FeeRate,
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
        envelope_pubkey: Option<XOnlyPublicKey>,
    ) -> Self {
        Self {
            magic_bytes,
            sequencer_address,
            reveal_amount,
            fee_rate,
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
}

impl EnvelopeData {
    pub fn new(
        commit_tx: Transaction,
        reveal_tx: Transaction,
        sighash: Buf32,
        reveal_script: ScriptBuf,
        taproot_spend_info: TaprootSpendInfo,
        envelope_pubkey: XOnlyPublicKey,
    ) -> Self {
        Self {
            commit_tx,
            reveal_tx,
            sighash,
            reveal_script,
            taproot_spend_info,
            envelope_pubkey,
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
    let (network, utxos, fee_rate) = fetch_envelope_prereqs(ctx)
        .await
        .map_err(EnvelopeError::PrereqFetch)?;
    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
        Some(envelope_pubkey),
    );
    create_envelope_transactions(&env_config, payload, utxos)
}

/// Builds envelope transactions using a temporary keypair and signs both commit and reveal
/// in-process.
///
/// Used when no external signer is required.
pub(crate) async fn build_and_sign_envelope_txs<R: Reader + Signer + Wallet>(
    payload: &L1Payload,
    ctx: &WriterContext<R>,
) -> Result<EnvelopeData, EnvelopeError> {
    let (network, utxos, fee_rate) = fetch_envelope_prereqs(ctx)
        .await
        .map_err(EnvelopeError::PrereqFetch)?;
    let keypair = generate_key_pair()?;
    let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
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
) -> anyhow::Result<(Network, Vec<ListUnspentItem>, FeeRate)> {
    let network = ctx.client.network().await?;
    let utxos = ctx
        .client
        .list_unspent(None, None, None, None, None)
        .await?
        .0;
    let fee_rate = resolve_fee_rate(ctx.client.as_ref(), ctx.config.as_ref()).await?;
    Ok((network, utxos, fee_rate))
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
    )?;

    // Build commit tx
    let (commit_tx, _) = build_commit_transaction(
        utxos,
        reveal_address,
        env_config.sequencer_address.clone(),
        commit_value,
        env_config.fee_rate,
    )?;

    let output_to_reveal = commit_tx.output[0].clone();

    // Build reveal tx
    let reveal_tx = build_reveal_transaction(
        commit_tx.clone(),
        env_config.sequencer_address.clone(),
        env_config.reveal_amount,
        env_config.fee_rate,
        &reveal_script,
        tag_script,
        &taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or(anyhow!("Cannot create control block".to_string()))?,
    )?;

    // Compute sighash for the reveal tx
    let sighash = compute_reveal_sighash(&reveal_tx, &output_to_reveal, &reveal_script)?;

    Ok(EnvelopeData::new(
        commit_tx,
        reveal_tx,
        sighash,
        reveal_script,
        taproot_spend_info,
        public_key,
    ))
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

/// Returns whether a wallet output can fund a commit transaction.
///
/// Outputs of exactly [`BITCOIN_DUST_LIMIT`] qualify: every reveal returns that amount to the
/// sequencer address, so the wallet accumulates outputs of that size.
pub(crate) fn is_commit_funding_utxo(utxo: &ListUnspentItem) -> bool {
    utxo.spendable
        && utxo.solvable
        && utxo.amount.to_sat() >= BITCOIN_DUST_LIMIT
        && is_supported_commit_utxo(utxo)
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
/// # Panics
///
/// Panics if a UTXO's script is neither P2WPKH nor P2TR. Callers select from outputs that pass
/// [`is_commit_funding_utxo`].
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
    fund_commit_transaction(
        utxos,
        vec![TxOut {
            script_pubkey: recipient.script_pubkey(),
            value: Amount::from_sat(output_value),
        }],
        change_address.script_pubkey(),
        fee_rate,
    )
}

/// Selects enough wallet outputs to fund `outputs` and the fee implied by their signed input size.
///
/// Returns the selected outputs, their total value, and that fee.
fn select_commit_utxos(
    utxos: &[ListUnspentItem],
    outputs: &[TxOut],
    base_output_total: u64,
    minimum_excess: u64,
    fee_rate: FeeRate,
) -> Result<(Vec<ListUnspentItem>, u64, u64), EnvelopeError> {
    // The outputs alone are a lower bound on the signed size.
    let mut estimated_size = signed_commit_vsize(&[], outputs);

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

        // A higher fee can select a smaller P2TR input in place of a P2WPKH one, so the size is
        // not monotonic in the fee. Each selection is checked against its own size, and the
        // estimate only moves when the selection was larger than estimated, so it only grows.
        if sum >= required {
            return Ok((chosen_utxos, sum, fee));
        }

        estimated_size = signed_size;
    }
}

/// Funds fixed commit outputs at the requested rate, adding change when it stays above dust.
///
/// Without room for change, the remainder goes to the fee.
pub(crate) fn fund_commit_transaction(
    utxos: Vec<ListUnspentItem>,
    base_outputs: Vec<TxOut>,
    change_script: ScriptBuf,
    fee_rate: FeeRate,
) -> Result<(Transaction, Vec<ListUnspentItem>), EnvelopeError> {
    let base_output_total = base_outputs
        .iter()
        .try_fold(0u64, |total, output| {
            total.checked_add(output.value.to_sat())
        })
        .ok_or(EnvelopeError::FeeOverflow)?;
    let utxos: Vec<ListUnspentItem> = utxos.into_iter().filter(is_commit_funding_utxo).collect();

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
            ));
        }
        Err(EnvelopeError::NotEnoughUtxos(_, _)) => {}
        Err(error) => return Err(error),
    }

    let (chosen_utxos, _, _) =
        select_commit_utxos(&utxos, &base_outputs, base_output_total, 0, fee_rate)?;
    Ok((
        Transaction {
            lock_time: LockTime::ZERO,
            version: Version(2),
            input: commit_inputs(&chosen_utxos),
            output: base_outputs,
        },
        chosen_utxos,
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
) -> Result<u64, EnvelopeError> {
    let reveal_vsize = get_size(
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
    );
    let fee = fee_sats_for_vsize(reveal_vsize, fee_rate)?;
    reveal_value
        .checked_add(fee)
        .ok_or(EnvelopeError::FeeOverflow)
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
    use std::{slice, sync::Arc};

    use bitcoin::{
        absolute::LockTime,
        script,
        secp256k1::{constants::SCHNORR_SIGNATURE_SIZE, Secp256k1, SecretKey},
        taproot::ControlBlock,
        transaction::Version,
        Address, Network, OutPoint, ScriptBuf, ScriptHash, Sequence, Transaction, TxIn, TxOut,
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

    /// Rewrites a mock UTXO as a P2TR key-path output.
    fn as_p2tr_utxo(mut utxo: ListUnspentItem) -> ListUnspentItem {
        let address = Address::p2tr(SECP256K1, test_envelope_pubkey(), None, Network::Regtest);
        utxo.script_pubkey = address.script_pubkey();
        utxo.address = address.as_unchecked().clone();
        utxo
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

        let inp_txn = get_txn_from_utxo(utxo, &ctx.sequencer_address);
        let mut tx = super::build_reveal_transaction(
            inp_txn,
            ctx.sequencer_address.clone(),
            ctx.config.reveal_amount,
            FeeRate::from_sat_per_vb_u32(8),
            &_reveal_script,
            tag_script.clone(),
            &control_block,
        )
        .unwrap();

        tx.input[0].witness.push([0; SCHNORR_SIGNATURE_SIZE]);
        tx.input[0].witness.push(_reveal_script.clone());
        tx.input[0].witness.push(control_block.serialize());

        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].previous_output.vout, utxo.vout);

        assert_eq!(tx.output.len(), 2);
        assert_eq!(tx.output[1].value.to_sat(), ctx.config.reveal_amount);
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
        let viable = utxos[2].clone();

        let (tx, consumed) = super::build_commit_transaction(
            vec![unspendable, unsolvable, viable.clone()],
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
    fn test_build_commit_transaction_uses_exact_dust_limit_utxos() {
        let (ctx, _, _, mut utxos) = get_mock_data();
        utxos.truncate(2);
        for utxo in &mut utxos {
            utxo.amount = Amount::from_sat(BITCOIN_DUST_LIMIT);
        }

        let (tx, consumed) = super::build_commit_transaction(
            utxos,
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            BITCOIN_DUST_LIMIT,
            FeeRate::from_sat_per_vb_u32(1),
        )
        .unwrap();

        assert_eq!(consumed.len(), 2);
        assert!(consumed
            .iter()
            .all(|utxo| utxo.amount.to_sat() == BITCOIN_DUST_LIMIT));
        assert_eq!(tx.input.len(), 2);
    }

    #[test]
    fn test_build_commit_transaction_skips_sub_dust_and_unsupported_utxos() {
        let (ctx, _, _, utxos) = get_mock_data();
        let mut sub_dust = utxos[0].clone();
        sub_dust.amount = Amount::from_sat(BITCOIN_DUST_LIMIT - 1);
        let mut nested_segwit = utxos[1].clone();
        nested_segwit.script_pubkey = ScriptBuf::new_p2sh(&ScriptHash::all_zeros());

        let res = super::build_commit_transaction(
            vec![sub_dust, nested_segwit],
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            BITCOIN_DUST_LIMIT,
            FeeRate::from_sat_per_vb_u32(1),
        );

        assert!(matches!(res, Err(EnvelopeError::NotEnoughUtxos(_, 0))));
    }

    #[test]
    fn test_build_commit_transaction_terminates_when_higher_fee_selects_smaller_input() {
        let (ctx, _, _, utxos) = get_mock_data();
        let fee_rate = FeeRate::from_sat_per_vb_u32(100);
        let output_value = 20_000;
        let outputs = [
            TxOut {
                value: Amount::from_sat(output_value),
                script_pubkey: ctx.sequencer_address.script_pubkey(),
            },
            TxOut {
                value: Amount::ZERO,
                script_pubkey: ctx.sequencer_address.script_pubkey(),
            },
        ];
        let p2tr = as_p2tr_utxo(utxos[0].clone());
        let mut p2wpkh = utxos[1].clone();
        let p2tr_fee = fee_sats_for_vsize(
            signed_commit_vsize(slice::from_ref(&p2tr), &outputs),
            fee_rate,
        )
        .unwrap();
        let p2wpkh_fee = fee_sats_for_vsize(
            signed_commit_vsize(slice::from_ref(&p2wpkh), &outputs),
            fee_rate,
        )
        .unwrap();
        // Priced as the P2TR spend, the P2WPKH output funds the commit with change. Priced as the
        // larger P2WPKH spend, it no longer does, and selection moves to the P2TR output.
        p2wpkh.amount = Amount::from_sat(output_value + p2tr_fee + BITCOIN_DUST_LIMIT);
        assert!(p2wpkh.amount.to_sat() < output_value + p2wpkh_fee);

        let (tx, consumed) = super::build_commit_transaction(
            vec![p2wpkh, p2tr.clone()],
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            output_value,
            fee_rate,
        )
        .unwrap();

        assert_eq!(consumed, vec![p2tr]);
        let output_total: u64 = tx.output.iter().map(|output| output.value.to_sat()).sum();
        let paid_fee = consumed[0].amount.to_sat() - output_total;
        let required_fee =
            fee_sats_for_vsize(signed_commit_vsize(&consumed, &tx.output), fee_rate).unwrap();
        assert!(paid_fee >= required_fee);
    }

    #[test]
    fn test_build_commit_transaction_drops_unaffordable_change() {
        let (ctx, _, _, utxos) = get_mock_data();
        let fee_rate = FeeRate::from_sat_per_vb_u32(100);
        let output_value = 20_000;
        let base_output = TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: ctx.sequencer_address.script_pubkey(),
        };
        let mut utxo = utxos[0].clone();
        let no_change_fee = fee_sats_for_vsize(
            signed_commit_vsize(slice::from_ref(&utxo), slice::from_ref(&base_output)),
            fee_rate,
        )
        .unwrap();
        // The remainder after the no-change fee is dust-sized, too small to also pay for a change
        // output.
        utxo.amount = Amount::from_sat(output_value + no_change_fee + BITCOIN_DUST_LIMIT);

        let (tx, consumed) = super::build_commit_transaction(
            vec![utxo.clone()],
            ctx.sequencer_address.clone(),
            ctx.sequencer_address.clone(),
            output_value,
            fee_rate,
        )
        .unwrap();

        assert_eq!(consumed, vec![utxo]);
        assert_eq!(tx.output, vec![base_output]);
    }

    #[test]
    fn test_signed_commit_vsize_counts_input_witnesses() {
        let (ctx, _, _, utxos) = get_mock_data();
        let outputs = [TxOut {
            value: Amount::from_sat(100_000),
            script_pubkey: ctx.sequencer_address.script_pubkey(),
        }];

        // 82 non-witness bytes (328 WU) plus the segwit marker and the input's witness: 111 WU
        // for a P2WPKH signature and public key, 68 WU for a P2TR key-path signature.
        let p2wpkh_vsize = signed_commit_vsize(&utxos[..1], &outputs);
        let p2tr_vsize = signed_commit_vsize(&[as_p2tr_utxo(utxos[0].clone())], &outputs);

        assert_eq!(p2wpkh_vsize, 110);
        assert_eq!(p2tr_vsize, 99);
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

        assert_eq!(
            unsigned.reveal_tx.input[0].previous_output.txid,
            unsigned.commit_tx.compute_txid(),
            "reveal should use commit as input"
        );
        assert_eq!(
            unsigned.reveal_tx.input[0].previous_output.vout, 0,
            "reveal should use commit as input"
        );

        assert_eq!(
            unsigned.reveal_tx.output[1].script_pubkey,
            ctx.sequencer_address.script_pubkey(),
            "reveal should pay to the correct address"
        );

        // Sighash should be non-zero
        assert_ne!(unsigned.sighash, Buf32::zero());
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
