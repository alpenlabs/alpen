use core::{result::Result::Ok, str::FromStr};
use std::{cmp::Reverse, sync::Arc};

use anyhow::anyhow;
use bitcoin::{
    absolute::LockTime,
    blockdata::{
        opcodes::{
            all::{OP_CHECKSIG, OP_ENDIF, OP_IF},
            OP_FALSE,
        },
        script,
    },
    hashes::Hash,
    key::{TapTweak, TweakedPublicKey, UntweakedKeypair},
    script::PushBytesBuf,
    secp256k1::{
        self, constants::SCHNORR_SIGNATURE_SIZE, schnorr::Signature, Secp256k1, XOnlyPublicKey,
    },
    sighash::{Prevouts, SighashCache},
    taproot::{
        ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootBuilderError,
        TaprootSpendInfo,
    },
    transaction::Version,
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid,
    Witness,
};
use rand::RngCore;
use thiserror::Error;

use crate::{
    rpc::{
        traits::{BitcoinReader, BitcoinSigner, BitcoinWallet},
        types::ListUnspent,
    },
    writer::config::{InscriptionFeePolicy, WriterConfig},
};

const BITCOIN_DUST_LIMIT: u64 = 546;

// TODO: these might need to be in rollup params
const BATCH_DATA_TAG: &[u8] = &[1];
const ROLLUP_NAME_TAG: &[u8] = &[3];

#[derive(Debug, Error)]
pub enum InscriptionError {
    #[error("Not enough UTXOs for transaction of {0} sats")]
    NotEnoughUtxos(u64),

    #[error("Error building taproot")]
    Taproot(#[from] TaprootBuilderError),

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub async fn build_inscription_txs(
    payload: &[u8],
    rpc_client: &Arc<impl BitcoinReader + BitcoinWallet + BitcoinSigner>,
    config: &WriterConfig,
) -> anyhow::Result<(Transaction, Transaction)> {
    // let (signature, pub_key) = sign_blob_with_private_key(&payload, &config.private_key)?;
    let network = rpc_client.network().await?;
    let utxos = rpc_client.get_utxos().await?;

    let fee_rate = match config.inscription_fee_policy {
        InscriptionFeePolicy::Smart => rpc_client.estimate_smart_fee(1).await? * 2,
        InscriptionFeePolicy::Fixed(val) => val,
    };
    create_inscription_transactions(
        &config.rollup_name,
        payload,
        utxos,
        config.sequencer_address.clone(),
        config.amount_for_reveal_txn,
        fee_rate,
        network,
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}

#[allow(clippy::too_many_arguments)]
pub fn create_inscription_transactions(
    rollup_name: &str,
    write_intent: &[u8],
    utxos: Vec<ListUnspent>,
    recipient: Address,
    reveal_value: u64,
    fee_rate: u64,
    network: Network,
) -> Result<(Transaction, Transaction), InscriptionError> {
    // Create commit key
    let secp256k1 = Secp256k1::new();
    let key_pair = generate_key_pair(&secp256k1)?;
    let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

    // Start creating inscription content
    let reveal_script = build_reveal_script(&public_key, rollup_name, write_intent)?;

    // Create spend info for tapscript
    let taproot_spend_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())?
        .finalize(&secp256k1, public_key)
        .map_err(|_| anyhow!("Could not build taproot spend info"))?;

    // Create reveal address
    let reveal_address = Address::p2tr(
        &secp256k1,
        public_key,
        taproot_spend_info.merkle_root(),
        network,
    );

    // Calculate commit value
    let commit_value = calculate_commit_output_value(
        &recipient,
        reveal_value,
        fee_rate,
        &reveal_script,
        &taproot_spend_info,
    );

    // Build commit tx
    let (unsigned_commit_tx, _) = build_commit_transaction(
        utxos,
        reveal_address.clone(),
        recipient.clone(),
        commit_value,
        fee_rate,
    )?;

    let output_to_reveal = unsigned_commit_tx.output[0].clone();

    // Build reveal tx
    let mut reveal_tx = build_reveal_transaction(
        unsigned_commit_tx.clone(),
        recipient,
        reveal_value,
        fee_rate,
        &reveal_script,
        &taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or(anyhow!("Cannot create control block".to_string()))?,
    )?;

    // Sign reveal tx
    sign_reveal_transaction(
        &secp256k1,
        &mut reveal_tx,
        &output_to_reveal,
        &reveal_script,
        &taproot_spend_info,
        &key_pair,
    )?;

    // Check if inscription is locked to the correct address
    assert_correct_address(
        &secp256k1,
        &key_pair,
        &taproot_spend_info,
        &reveal_address,
        network,
    );

    Ok((unsigned_commit_tx, reveal_tx))
}

fn get_size(
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
        tx.input[i].witness.push(
            Signature::from_slice(&[0; SCHNORR_SIGNATURE_SIZE])
                .unwrap()
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
fn choose_utxos(
    utxos: &[ListUnspent],
    amount: u64,
) -> Result<(Vec<ListUnspent>, u64), InscriptionError> {
    let mut bigger_utxos: Vec<&ListUnspent> = utxos
        .iter()
        .filter(|utxo| utxo.amount.to_sat() >= amount)
        .collect();
    let mut sum = 0;

    if !bigger_utxos.is_empty() {
        // sort vec by amount (small first)
        bigger_utxos.sort_by_key(|&x| x.amount);

        // single utxo will be enough
        // so return the transaction
        let utxo = bigger_utxos[0];
        sum += utxo.amount.to_sat();

        Ok((vec![utxo.clone()], sum))
    } else {
        let mut smaller_utxos: Vec<&ListUnspent> = utxos
            .iter()
            .filter(|utxo| utxo.amount.to_sat() < amount)
            .collect();

        // sort vec by amount (large first)
        smaller_utxos.sort_by_key(|x| Reverse(&x.amount));

        let mut chosen_utxos: Vec<ListUnspent> = vec![];

        for utxo in smaller_utxos {
            sum += utxo.amount.to_sat();
            chosen_utxos.push(utxo.clone());

            if sum >= amount {
                break;
            }
        }

        if sum < amount {
            return Err(InscriptionError::NotEnoughUtxos(amount));
        }

        Ok((chosen_utxos, sum))
    }
}

fn build_commit_transaction(
    utxos: Vec<ListUnspent>,
    recipient: Address,
    change_address: Address,
    output_value: u64,
    fee_rate: u64,
) -> Result<(Transaction, Vec<ListUnspent>), InscriptionError> {
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

    let utxos: Vec<ListUnspent> = utxos
        .iter()
        .filter(|utxo| utxo.spendable && utxo.solvable && utxo.amount.to_sat() > BITCOIN_DUST_LIMIT)
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
            txid: Txid::from_str(
                "0000000000000000000000000000000000000000000000000000000000000000",
            )
            .unwrap(),
            vout: 0,
        },
        script_sig: script::Builder::new().into_script(),
        witness: Witness::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
    }]
}

fn build_reveal_transaction(
    input_transaction: Transaction,
    recipient: Address,
    output_value: u64,
    fee_rate: u64,
    reveal_script: &ScriptBuf,
    control_block: &ControlBlock,
) -> Result<Transaction, InscriptionError> {
    let outputs: Vec<TxOut> = vec![TxOut {
        value: Amount::from_sat(output_value),
        script_pubkey: recipient.script_pubkey(),
    }];

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
        return Err(InscriptionError::NotEnoughUtxos(input_required.to_sat()));
    }
    let tx = Transaction {
        lock_time: LockTime::ZERO,
        version: Version(2),
        input: inputs,
        output: outputs,
    };

    Ok(tx)
}

pub fn generate_key_pair(
    secp256k1: &Secp256k1<secp256k1::All>,
) -> Result<UntweakedKeypair, anyhow::Error> {
    let mut rand_bytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut rand_bytes);
    Ok(UntweakedKeypair::from_seckey_slice(secp256k1, &rand_bytes)?)
}

fn build_reveal_script(
    taproot_public_key: &XOnlyPublicKey,
    rollup_name: &str,
    write_intent: &[u8],
) -> Result<script::ScriptBuf, anyhow::Error> {
    let mut builder = script::Builder::new()
        .push_x_only_key(taproot_public_key)
        .push_opcode(OP_CHECKSIG)
        .push_opcode(OP_FALSE)
        .push_opcode(OP_IF)
        .push_slice(PushBytesBuf::try_from(ROLLUP_NAME_TAG.to_vec())?)
        .push_slice(PushBytesBuf::try_from(rollup_name.as_bytes().to_vec())?)
        // .push_slice(PushBytesBuf::try_from(SIGNATURE_TAG.to_vec())?)
        // .push_slice(PushBytesBuf::try_from(seq_signature)?)
        // .push_slice(PushBytesBuf::try_from(PUBLICKEY_TAG.to_vec())?)
        // // Pubkey corresponding to the above signature
        // .push_slice(PushBytesBuf::try_from(seq_public_key)?)
        .push_slice(PushBytesBuf::try_from(BATCH_DATA_TAG.to_vec())?)
        .push_int(write_intent.len() as i64);

    for chunk in write_intent.chunks(520) {
        builder = builder.push_slice(PushBytesBuf::try_from(chunk.to_vec())?);
    }

    Ok(builder.push_opcode(OP_ENDIF).into_script())
}

fn calculate_commit_output_value(
    recipient: &Address,
    reveal_value: u64,
    fee_rate: u64,
    reveal_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
) -> u64 {
    get_size(
        &default_txin(),
        &[TxOut {
            script_pubkey: recipient.script_pubkey(),
            value: Amount::from_sat(reveal_value),
        }],
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

fn sign_reveal_transaction(
    secp256k1: &Secp256k1<secp256k1::All>,
    reveal_tx: &mut Transaction,
    output_to_reveal: &TxOut,
    reveal_script: &script::ScriptBuf,
    taproot_spend_info: &TaprootSpendInfo,
    key_pair: &UntweakedKeypair,
) -> Result<(), anyhow::Error> {
    let mut sighash_cache = SighashCache::new(reveal_tx);
    let signature_hash = sighash_cache.taproot_script_spend_signature_hash(
        0,
        &Prevouts::All(&[output_to_reveal]),
        TapLeafHash::from_script(reveal_script, LeafVersion::TapScript),
        bitcoin::sighash::TapSighashType::Default,
    )?;

    let mut randbytes = [0; 32];
    rand::thread_rng().fill_bytes(&mut randbytes);

    let signature = secp256k1.sign_schnorr_with_aux_rand(
        &secp256k1::Message::from_digest_slice(signature_hash.as_byte_array())?,
        key_pair,
        &randbytes,
    );

    let witness = sighash_cache.witness_mut(0).unwrap();
    witness.push(signature.as_ref());
    witness.push(reveal_script);
    witness.push(
        taproot_spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .ok_or(anyhow!("Could not create control block"))?
            .serialize(),
    );

    Ok(())
}

fn assert_correct_address(
    secp256k1: &Secp256k1<secp256k1::All>,
    key_pair: &UntweakedKeypair,
    taproot_spend_info: &TaprootSpendInfo,
    commit_tx_address: &Address,
    network: Network,
) {
    let recovery_key_pair = key_pair.tap_tweak(secp256k1, taproot_spend_info.merkle_root());
    let x_only_pub_key = recovery_key_pair.to_inner().x_only_public_key().0;
    assert_eq!(
        Address::p2tr_tweaked(
            TweakedPublicKey::dangerous_assume_tweaked(x_only_pub_key),
            network,
        ),
        *commit_tx_address
    );
}

#[cfg(test)]
mod tests {
    use core::str::FromStr;

    use bitcoin::{
        absolute::LockTime, script, secp256k1::constants::SCHNORR_SIGNATURE_SIZE,
        taproot::ControlBlock, Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn,
        TxOut, Witness,
    };

    use super::*;
    use crate::{rpc::types::ListUnspent, writer::builder::InscriptionError};

    const BTC_TO_SATS: u64 = 100_000_000;
    const REVEAL_OUTPUT_AMOUNT: u64 = BITCOIN_DUST_LIMIT;

    #[allow(clippy::type_complexity)]
    fn get_mock_data() -> (
        &'static str,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Address,
        Vec<ListUnspent>,
    ) {
        let rollup_name = "test_rollup";
        let body = vec![100; 1000];
        let signature = vec![100; 64];
        let sequencer_public_key = vec![100; 33];
        let address =
            Address::from_str("bc1pp8qru0ve43rw9xffmdd8pvveths3cx6a5t6mcr0xfn9cpxx2k24qf70xq9")
                .unwrap();

        let utxos = vec![
            ListUnspent {
                txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.clone(),
                script_pubkey: "foo".to_string(),
                amount: Amount::from_btc(100.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: None,
                safe: true,
            },
            ListUnspent {
                txid: "44990141674ff56ed6fee38879e497b2a726cddefd5e4d9b7bf1c4e561de4347"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.clone(),
                script_pubkey: "foo".to_string(),
                amount: Amount::from_btc(50.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: None,
                safe: true,
            },
            ListUnspent {
                txid: "4dbe3c10ee0d6bf16f9417c68b81e963b5bccef3924bbcb0885c9ea841912325"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.clone(),
                script_pubkey: "foo".to_string(),
                amount: Amount::from_btc(10.0).unwrap(),
                confirmations: 100,
                spendable: true,
                solvable: true,
                label: None,
                safe: true,
            },
        ];

        (
            rollup_name,
            body,
            signature,
            sequencer_public_key,
            address.assume_checked(),
            utxos,
        )
    }

    #[test]
    fn choose_utxos() {
        let (_, _, _, _, _, utxos) = get_mock_data();

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
            Err(InscriptionError::NotEnoughUtxos(50_000_000_000))
        ));
    }

    fn get_txn_from_utxo(utxo: &ListUnspent, _address: &Address) -> Transaction {
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
            version: bitcoin::transaction::Version(2),
            input: inputs,
            output: outputs,
        }
    }

    #[test]
    fn test_build_reveal_transaction() {
        let (_, _, _, _, address, utxos) = get_mock_data();

        let utxo = utxos.first().unwrap();
        let _script = ScriptBuf::from_hex("62a58f2674fd840b6144bea2e63ebd35c16d7fd40252a2f28b2a01a648df356343e47976d7906a0e688bf5e134b6fd21bd365c016b57b1ace85cf30bf1206e27").unwrap();
        let control_block = ControlBlock::decode(&[
            193, 165, 246, 250, 6, 222, 28, 9, 130, 28, 217, 67, 171, 11, 229, 62, 48, 206, 219,
            111, 155, 208, 6, 7, 119, 63, 146, 90, 227, 254, 231, 232, 249,
        ])
        .unwrap(); // should be 33 bytes

        let inp_txn = get_txn_from_utxo(utxo, &address);
        let mut tx = super::build_reveal_transaction(
            inp_txn,
            address.clone(),
            REVEAL_OUTPUT_AMOUNT,
            8,
            &_script,
            &control_block,
        )
        .unwrap();

        tx.input[0].witness.push([0; SCHNORR_SIGNATURE_SIZE]);
        tx.input[0].witness.push(_script.clone());
        tx.input[0].witness.push(control_block.serialize());

        assert_eq!(tx.input.len(), 1);
        assert_eq!(tx.input[0].previous_output.vout, utxo.vout);

        assert_eq!(tx.output.len(), 1);
        assert_eq!(tx.output[0].value.to_sat(), REVEAL_OUTPUT_AMOUNT);
        assert_eq!(tx.output[0].script_pubkey, address.script_pubkey());

        // Test not enough utxos
        let utxo = utxos.get(2).unwrap();
        let inp_txn = get_txn_from_utxo(utxo, &address);
        let inp_required = 5000000000;
        let tx = super::build_reveal_transaction(
            inp_txn,
            address.clone(),
            inp_required,
            750,
            &_script,
            &control_block,
        );

        assert!(tx.is_err());
        assert!(matches!(tx, Err(InscriptionError::NotEnoughUtxos(_))));
    }

    #[test]
    fn test_create_inscription_transactions() {
        let (rollup_name, _, _, _, address, utxos) = get_mock_data();

        let write_intent = vec![0u8; 100];
        let (commit, reveal) = super::create_inscription_transactions(
            rollup_name,
            &write_intent,
            utxos.to_vec(),
            address.clone(),
            REVEAL_OUTPUT_AMOUNT,
            10,
            bitcoin::Network::Bitcoin,
        )
        .unwrap();

        // check outputs
        assert_eq!(commit.output.len(), 2, "commit tx should have 2 outputs");

        assert_eq!(reveal.output.len(), 1, "reveal tx should have 1 output");

        assert_eq!(
            commit.input[0].previous_output.txid, utxos[2].txid,
            "utxo to inscribe should be chosen correctly"
        );
        assert_eq!(
            commit.input[0].previous_output.vout, utxos[2].vout,
            "utxo to inscribe should be chosen correctly"
        );

        assert_eq!(
            reveal.input[0].previous_output.txid,
            commit.compute_txid(),
            "reveal should use commit as input"
        );
        assert_eq!(
            reveal.input[0].previous_output.vout, 0,
            "reveal should use commit as input"
        );

        assert_eq!(
            reveal.output[0].script_pubkey,
            address.script_pubkey(),
            "reveal should pay to the correct address"
        );
    }

    // TODO: make the tests more comprehensive
}
