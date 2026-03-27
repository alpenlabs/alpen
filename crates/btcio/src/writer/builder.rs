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
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
};
use bitcoind_async_client::{
    corepc_types::model::ListUnspentItem,
    traits::{Reader, Signer, Wallet},
};
use rand::{rngs::OsRng, RngCore};
use strata_csm_types::L1Payload;
use strata_l1_envelope_fmt::{builder::EnvelopeScriptBuilder, errors::EnvelopeBuildError};
use strata_l1_txfmt::{self, MagicBytes, ParseConfig, TxFmtError};
use strata_primitives::buf::Buf32;
use thiserror::Error;

use super::context::WriterContext;
use crate::writer::fees::resolve_fee_rate;

pub(crate) const BITCOIN_DUST_LIMIT: u64 = 546;

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
    // TODO: Make this and all other bitcoin related values to Amount
    pub reveal_amount: u64,
    /// Bitcoin network
    pub network: Network,
    /// Bitcoin fee rate, sats/vByte
    pub fee_rate: u64,
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
        fee_rate: u64,
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

// TODO: these might need to be in rollup params
#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("no payload provided")]
    EmptyPayload,

    #[error("insufficient funds for tx (need {0} sats, have {1} sats)")]
    NotEnoughUtxos(u64, u64),

    #[error("Could not sign raw transaction: {0}")]
    SignRawTransaction(String),

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

/// Intermediate data from building unsigned envelope transactions.
///
/// Held in memory by the watcher task. Lost on restart, which triggers a rebuild
/// from `Unsigned` status.
#[derive(Debug, Clone)]
pub struct UnsignedEnvelopeData {
    /// The unsigned commit transaction.
    pub commit_tx: Transaction,
    /// The unsigned reveal transaction (no witness yet).
    pub reveal_tx: Transaction,
    /// The taproot script-spend sighash that needs to be signed.
    pub sighash: Buf32,
    /// The reveal script used in the taproot leaf.
    pub reveal_script: ScriptBuf,
    /// The taproot spend info for constructing the witness.
    pub taproot_spend_info: TaprootSpendInfo,
}

// This is hacky solution. As `btcio` has `transaction builder` that `tx-parser` depends on. But
// Btcio depends on `tx-parser`. So this file is behind a feature flag 'test-utils' and on dev
// dependencies on `tx-parser`, we include {btcio, feature="strata_test_utils"} , so cyclic
// dependency doesn't happen
pub(crate) async fn build_envelope_txs<R: Reader + Signer + Wallet>(
    payload: &L1Payload,
    ctx: &WriterContext<R>,
) -> anyhow::Result<UnsignedEnvelopeData> {
    let network = ctx.client.network().await?;
    let utxos = ctx
        .client
        .list_unspent(None, None, None, None, None)
        .await?
        .0;

    let fee_rate = resolve_fee_rate(ctx.client.as_ref(), ctx.config.as_ref()).await?;
    let envelope_pubkey = ctx
        .envelope_pubkey
        .ok_or_else(|| anyhow::anyhow!("envelope_pubkey is required for envelope transactions"))?;
    let env_config = EnvelopeConfig::new(
        ctx.btcio_params.magic_bytes(),
        ctx.sequencer_address.clone(),
        network,
        fee_rate,
        BITCOIN_DUST_LIMIT,
        Some(envelope_pubkey),
    );
    create_envelope_transactions(&env_config, payload, utxos)
        .map_err(|e| anyhow::anyhow!(e.to_string()))
}

/// Builds unsigned envelope transactions (commit + reveal) and computes the sighash.
///
/// Returns an [`UnsignedEnvelopeData`] containing the transactions and intermediate data
/// needed to attach the signature later via [`attach_reveal_signature`].
pub fn create_envelope_transactions(
    env_config: &EnvelopeConfig,
    payload: &L1Payload,
    utxos: Vec<ListUnspentItem>,
) -> Result<UnsignedEnvelopeData, EnvelopeError> {
    let public_key = env_config
        .envelope_pubkey
        .ok_or_else(|| anyhow!("envelope_pubkey is required for single-envelope transactions"))?;

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
    );

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

    Ok(UnsignedEnvelopeData {
        commit_tx,
        reveal_tx,
        sighash,
        reveal_script,
        taproot_spend_info,
    })
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

/// Choose utxos almost naively.
pub(crate) fn choose_utxos(
    utxos: &[ListUnspentItem],
    amount: u64,
) -> Result<(Vec<ListUnspentItem>, u64), EnvelopeError> {
    let mut bigger_utxos: Vec<&ListUnspentItem> = utxos
        .iter()
        .filter(|utxo| utxo.amount.to_sat() >= amount as i64)
        .collect();
    let mut sum: u64 = 0;

    if !bigger_utxos.is_empty() {
        // sort vec by amount (small first)
        bigger_utxos.sort_by_key(|&x| x.amount);

        // single utxo will be enough
        // so return the transaction
        let utxo = bigger_utxos[0];
        sum += utxo.amount.to_sat() as u64;

        Ok((vec![utxo.clone()], sum))
    } else {
        let mut smaller_utxos: Vec<&ListUnspentItem> = utxos
            .iter()
            .filter(|utxo| utxo.amount.to_sat() < amount as i64)
            .collect();

        // sort vec by amount (large first)
        smaller_utxos.sort_by_key(|x| Reverse(&x.amount));

        let mut chosen_utxos: Vec<ListUnspentItem> = vec![];

        for utxo in smaller_utxos {
            sum += utxo.amount.to_sat() as u64;
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
    fee_rate: u64,
) -> Result<(Transaction, Vec<ListUnspentItem>), EnvelopeError> {
    // get single input single output transaction size
    let mut size = get_size(
        &default_txin(),
        &[TxOut {
            script_pubkey: recipient.script_pubkey(),
            value: Amount::from_sat(output_value),
        }],
        None,
        None,
    );
    let mut last_size = size;

    let utxos: Vec<ListUnspentItem> = utxos
        .iter()
        .filter(|utxo| {
            utxo.spendable && utxo.solvable && utxo.amount.to_sat() > BITCOIN_DUST_LIMIT as i64
        })
        .cloned()
        .collect();

    let (commit_txn, consumed_utxo) = loop {
        let fee = (last_size as u64) * fee_rate;

        let input_total = output_value + fee;

        let res = choose_utxos(&utxos, input_total)?;

        let (chosen_utxos, sum) = res;

        let mut outputs: Vec<TxOut> = vec![];
        outputs.push(TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: recipient.script_pubkey(),
        });

        let mut direct_return = false;
        if let Some(excess) = sum.checked_sub(input_total) {
            if excess >= BITCOIN_DUST_LIMIT {
                outputs.push(TxOut {
                    value: Amount::from_sat(excess),
                    script_pubkey: change_address.script_pubkey(),
                });
            } else {
                // if dust is left, leave it for fee
                direct_return = true;
            }
        }

        let inputs: Vec<TxIn> = chosen_utxos
            .iter()
            .map(|u| TxIn {
                previous_output: OutPoint {
                    txid: u.txid,
                    vout: u.vout,
                },
                script_sig: script::Builder::new().into_script(),
                witness: Witness::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            })
            .collect();

        size = get_size(&inputs, &outputs, None, None);

        if size == last_size || direct_return {
            let commit_txn = Transaction {
                lock_time: LockTime::ZERO,
                version: Version(2),
                input: inputs,
                output: outputs,
            };

            break (commit_txn, chosen_utxos);
        }

        last_size = size;
    };

    Ok((commit_txn, consumed_utxo))
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
    fee_rate: u64,
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
    let fee = (size as u64) * fee_rate;
    let input_required = Amount::from_sat(output_value + fee);
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
    fee_rate: u64,
    reveal_script: &script::ScriptBuf,
    tag_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
) -> u64 {
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
    ) as u64
        * fee_rate
        + reveal_value
}

/// Generates a random keypair for envelope construction.
///
/// Used by the chunked envelope path which creates per-reveal ephemeral keypairs.
pub fn generate_key_pair() -> Result<UntweakedKeypair, anyhow::Error> {
    let mut rand_bytes = [0; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    Ok(UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes)?)
}

/// Signs and attaches a taproot script-spend witness to the reveal transaction.
///
/// Used by the chunked envelope path which signs in-process with ephemeral keypairs.
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
    use std::sync::Arc;

    use bitcoin::{
        absolute::LockTime, script, secp256k1::constants::SCHNORR_SIGNATURE_SIZE,
        taproot::ControlBlock, transaction::Version, Address, Network, OutPoint, ScriptBuf,
        Sequence, SignedAmount, Transaction, TxIn, TxOut, Witness,
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
                script_pubkey: ScriptBuf::new(),
                amount: SignedAmount::from_btc(100.0).unwrap(),
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
                script_pubkey: ScriptBuf::new(),
                amount: SignedAmount::from_btc(50.0).unwrap(),
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
                script_pubkey: ScriptBuf::new(),
                amount: SignedAmount::from_btc(10.0).unwrap(),
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
            value: utxo.amount.to_unsigned().unwrap(),
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
            8,
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
            750,
            &_reveal_script,
            tag_script,
            &control_block,
        );

        assert!(tx.is_err());
        assert!(matches!(tx, Err(EnvelopeError::NotEnoughUtxos(_, _))));
    }

    #[test]
    fn test_create_envelope_transactions() {
        let (ctx, _, _, utxos) = get_mock_data();

        let tag = TagData::new(1, 1, vec![]).unwrap();
        // Use 150 bytes to meet minimum envelope payload size of 126 bytes
        let payload = L1Payload::new(vec![vec![0u8; 150]], tag);

        use bitcoin::secp256k1::{Secp256k1, SecretKey};
        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[0x01; 32]).unwrap();
        let (pubkey, _) = sk.x_only_public_key(&secp);
        let env_config = EnvelopeConfig::new(
            MagicBytes::new(*b"ALPN"),
            ctx.sequencer_address.clone(),
            Network::Regtest,
            1000,
            546,
            Some(pubkey),
        );
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

    // TODO: make the tests more comprehensive
}
