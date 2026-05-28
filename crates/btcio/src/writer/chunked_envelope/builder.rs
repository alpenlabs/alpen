//! Builds chunked envelope transactions: one commit tx with N P2TR outputs
//! and a leading OP_RETURN, each P2TR spent by an independent reveal tx that
//! carries one chunk under
//! `<sequencer_pk> OP_CHECKSIG OP_FALSE OP_IF <chunk> OP_ENDIF`.
//!
//! Reveals are independent across batches: fee bumping a single reveal does
//! not cascade. Chunk ordering is implicit in commit-output ordering.

use core::{iter, slice};

use anyhow::anyhow;
use bitcoin::{
    absolute::LockTime,
    blockdata::script,
    hashes::Hash,
    key::Keypair,
    secp256k1::{XOnlyPublicKey, SECP256K1},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use bitcoind_async_client::corepc_types::model::ListUnspentItem;
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_l1_txfmt::MagicBytes;

use super::commit_op_return::build_commit_op_return;
use crate::writer::builder::{
    choose_utxos, get_size, sign_reveal_transaction, EnvelopeConfig, EnvelopeError,
    BITCOIN_DUST_LIMIT,
};

/// Intermediate state for each reveal before tx construction.
struct RevealArtifact {
    reveal_script: ScriptBuf,
    spend_info: TaprootSpendInfo,
    commit_value: u64,
}

/// One unsigned commit tx and N signed reveal txs.
#[derive(Debug)]
pub(crate) struct ChunkedEnvelopeTxs {
    pub commit_tx: Transaction,
    pub reveal_txs: Vec<Transaction>,
}

/// Builds a chunked envelope from raw chunk payloads.
///
/// Constructs one commit tx with `[OP_RETURN, P2TR_0, ..., P2TR_{N-1}, change?]`
/// and N reveal txs, each spending one P2TR output via a tapscript spend
/// signed under `sequencer_keypair`. The reveal tapscript shape is
/// `<sequencer_pk> OP_CHECKSIG OP_FALSE OP_IF <chunk_i_bytes> OP_ENDIF`.
///
/// The SPS-51 envelope framing applied to reveals lets the sigCheck transitively
/// authenticate the commit whose outputs the reveals spend.
pub(crate) fn build_chunked_envelope_txs(
    config: &EnvelopeConfig,
    chunks: &[Vec<u8>],
    magic_bytes: &MagicBytes,
    da_blob_version: u32,
    sequencer_keypair: &Keypair,
    utxos: Vec<ListUnspentItem>,
) -> Result<ChunkedEnvelopeTxs, EnvelopeError> {
    if chunks.is_empty() {
        return Err(EnvelopeError::EmptyPayload);
    }

    // The guest currently infers reveal count from the consecutive P2TR outputs
    // after the commit OP_RETURN. P2TR change would extend that run and make the
    // guest require a non-existent reveal.
    if config.sequencer_address.script_pubkey().is_p2tr() {
        return Err(EnvelopeError::P2trChangeAddressUnsupported);
    }

    let sequencer_xonly = XOnlyPublicKey::from_keypair(sequencer_keypair).0;
    let commit_op_return = build_commit_op_return(magic_bytes, da_blob_version);

    let mut artifacts = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let reveal_script = EnvelopeScriptBuilder::with_pubkey(&sequencer_xonly.serialize())?
            .add_envelopes(slice::from_ref(chunk))?
            .build_without_min_check()?;

        let spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())?
            .finalize(SECP256K1, sequencer_xonly)
            .map_err(|_| anyhow!("could not finalize taproot spend info"))?;

        // A reveal has one sequencer-address output plus its tapscript spend.
        let commit_value = calculate_reveal_commit_value(
            &config.sequencer_address,
            config.reveal_amount,
            config.fee_rate,
            &reveal_script,
            &spend_info,
        );

        artifacts.push(RevealArtifact {
            reveal_script,
            spend_info,
            commit_value,
        });
    }

    let commit_tx = build_multi_output_commit(config, &artifacts, &commit_op_return, utxos)?;
    let commit_txid = commit_tx.compute_txid();

    let mut reveal_txs = Vec::with_capacity(artifacts.len());

    // Reveals spend commit outputs starting at vout=1 (vout=0 is the OP_RETURN).
    for (i, artifact) in artifacts.iter().enumerate() {
        let vout = (i + 1) as u32;
        let commit_output = &commit_tx.output[vout as usize];

        let control_block = artifact
            .spend_info
            .control_block(&(artifact.reveal_script.clone(), LeafVersion::TapScript))
            .ok_or_else(|| anyhow!("cannot create control block for reveal {i}"))?;

        // Verify the commit output covers reveal fee + dust.
        let reveal_vsize = get_size(
            &[make_txin(commit_txid, vout)],
            &[TxOut {
                value: Amount::from_sat(config.reveal_amount),
                script_pubkey: config.sequencer_address.script_pubkey(),
            }],
            Some(&artifact.reveal_script),
            Some(&control_block),
        );
        let required = config.reveal_amount + (reveal_vsize as u64) * config.fee_rate;
        if commit_output.value < Amount::from_sat(required) {
            return Err(EnvelopeError::NotEnoughUtxos(
                required,
                commit_output.value.to_sat(),
            ));
        }

        let mut reveal_tx = Transaction {
            lock_time: LockTime::ZERO,
            version: Version(2),
            input: vec![make_txin(commit_txid, vout)],
            output: vec![TxOut {
                value: Amount::from_sat(config.reveal_amount),
                script_pubkey: config.sequencer_address.script_pubkey(),
            }],
        };

        sign_reveal_transaction(
            &mut reveal_tx,
            commit_output,
            &artifact.reveal_script,
            &artifact.spend_info,
            sequencer_keypair,
        )?;

        reveal_txs.push(reveal_tx);
    }

    Ok(ChunkedEnvelopeTxs {
        commit_tx,
        reveal_txs,
    })
}

/// Sizes the commit output funding one reveal tx.
///
/// The returned value is `vsize(reveal) * fee_rate + reveal_amount`.
///
/// This is the amount the matching commit P2TR output must hold so the reveal
/// can pay its fee plus the sequencer dust output.
fn calculate_reveal_commit_value(
    sequencer_address: &Address,
    reveal_amount: u64,
    fee_rate: u64,
    reveal_script: &ScriptBuf,
    spend_info: &TaprootSpendInfo,
) -> u64 {
    let reveal_output = TxOut {
        value: Amount::from_sat(reveal_amount),
        script_pubkey: sequencer_address.script_pubkey(),
    };
    let control_block = spend_info
        .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
        .expect("control block exists for the script we just built");
    let reveal_vsize = get_size(
        &[make_txin(bitcoin::Txid::all_zeros(), 0)],
        &[reveal_output],
        Some(reveal_script),
        Some(&control_block),
    ) as u64;
    reveal_amount + reveal_vsize * fee_rate
}

/// Builds the commit tx with `[OP_RETURN, P2TR_0, ..., P2TR_{N-1}, change?]`.
fn build_multi_output_commit(
    config: &EnvelopeConfig,
    artifacts: &[RevealArtifact],
    op_return_script: &ScriptBuf,
    utxos: Vec<ListUnspentItem>,
) -> Result<Transaction, EnvelopeError> {
    let spendable: Vec<ListUnspentItem> = utxos
        .into_iter()
        .filter(|u| u.spendable && u.solvable && u.amount.to_sat() > BITCOIN_DUST_LIMIT)
        .collect();

    let p2tr_outputs: Vec<TxOut> = artifacts
        .iter()
        .map(|a| {
            let addr = Address::p2tr(
                SECP256K1,
                a.spend_info.internal_key(),
                a.spend_info.merkle_root(),
                config.network,
            );
            TxOut {
                value: Amount::from_sat(a.commit_value),
                script_pubkey: addr.script_pubkey(),
            }
        })
        .collect();

    // Outputs are: [OP_RETURN, P2TR_0..N-1, change?]
    let op_return_output = TxOut {
        value: Amount::from_sat(0),
        script_pubkey: op_return_script.clone(),
    };

    let total_output: u64 = artifacts.iter().map(|a| a.commit_value).sum();

    let initial_outputs: Vec<TxOut> = iter::once(op_return_output.clone())
        .chain(p2tr_outputs.iter().cloned())
        .collect();

    let mut last_size = get_size(
        &[make_txin(bitcoin::Txid::all_zeros(), 0)],
        &initial_outputs,
        None,
        None,
    );

    loop {
        let fee = (last_size as u64) * config.fee_rate;
        let needed = total_output + fee;
        let (chosen, sum) = choose_utxos(&spendable, needed)?;

        let inputs: Vec<TxIn> = chosen.iter().map(|u| make_txin(u.txid, u.vout)).collect();

        let mut outputs: Vec<TxOut> = iter::once(op_return_output.clone())
            .chain(p2tr_outputs.iter().cloned())
            .collect();

        let mut done = false;
        if let Some(excess) = sum.checked_sub(needed) {
            if excess >= BITCOIN_DUST_LIMIT {
                outputs.push(TxOut {
                    value: Amount::from_sat(excess),
                    script_pubkey: config.sequencer_address.script_pubkey(),
                });
            } else {
                done = true;
            }
        }

        let size = get_size(&inputs, &outputs, None, None);
        if size == last_size || done {
            return Ok(Transaction {
                lock_time: LockTime::ZERO,
                version: Version(2),
                input: inputs,
                output: outputs,
            });
        }
        last_size = size;
    }
}

fn make_txin(txid: bitcoin::Txid, vout: u32) -> TxIn {
    TxIn {
        previous_output: OutPoint { txid, vout },
        script_sig: script::Builder::new().into_script(),
        witness: Witness::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        opcodes::all::OP_RETURN,
        secp256k1::{rand, Keypair, Secp256k1},
        Network, ScriptBuf, Txid,
    };
    use bitcoind_async_client::corepc_types::model::ListUnspentItem;

    use super::*;
    use crate::{
        test_utils::test_context::get_writer_context,
        writer::chunked_envelope::commit_op_return::COMMIT_OP_RETURN_PAYLOAD_LEN,
    };

    const TEST_DA_BLOB_VERSION: u32 = 1;

    fn test_keypair() -> Keypair {
        let secp = Secp256k1::new();
        Keypair::new(&secp, &mut rand::thread_rng())
    }

    fn get_mock_utxos() -> Vec<ListUnspentItem> {
        let ctx = get_writer_context();
        let address = ctx.sequencer_address.clone();
        vec![
            ListUnspentItem {
                txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                    .parse::<Txid>()
                    .unwrap(),
                vout: 0,
                address: address.as_unchecked().clone(),
                script_pubkey: ScriptBuf::new(),
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
                script_pubkey: ScriptBuf::new(),
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
        ]
    }

    fn get_test_config() -> EnvelopeConfig {
        let ctx = get_writer_context();
        EnvelopeConfig::new(
            ctx.btcio_params.magic_bytes,
            ctx.sequencer_address.clone(),
            Network::Regtest,
            1000,
            546,
            None,
        )
    }

    #[test]
    fn commit_marker_orders_reveals() {
        let config = get_test_config();
        let utxos = get_mock_utxos();
        let chunks = vec![vec![1u8; 150], vec![2u8; 150], vec![3u8; 150]];
        let magic = MagicBytes::from([0xAA, 0xBB, 0xCC, 0xDD]);
        let kp = test_keypair();

        let result =
            build_chunked_envelope_txs(&config, &chunks, &magic, TEST_DA_BLOB_VERSION, &kp, utxos)
                .unwrap();

        // commit: OP_RETURN + 3 P2TR + change = 5 outputs.
        assert_eq!(result.commit_tx.output.len(), 5);

        let op_return_script = &result.commit_tx.output[0].script_pubkey;
        let op_return_bytes = op_return_script.as_bytes();
        assert_eq!(
            op_return_bytes,
            [
                OP_RETURN.to_u8(),
                COMMIT_OP_RETURN_PAYLOAD_LEN as u8,
                0xAA,
                0xBB,
                0xCC,
                0xDD,
                0,
                0,
                0,
                TEST_DA_BLOB_VERSION as u8,
            ]
        );

        // reveals: each spends commit output i+1, has a single sequencer-dust output.
        assert_eq!(result.reveal_txs.len(), 3);
        let commit_txid = result.commit_tx.compute_txid();
        for (i, reveal) in result.reveal_txs.iter().enumerate() {
            assert_eq!(reveal.input[0].previous_output.txid, commit_txid);
            assert_eq!(reveal.input[0].previous_output.vout, (i + 1) as u32);
            assert_eq!(
                reveal.output.len(),
                1,
                "reveal carries one sequencer-dust output"
            );
            assert_eq!(
                reveal.output[0].script_pubkey,
                config.sequencer_address.script_pubkey()
            );
        }
    }

    #[test]
    fn build_chunked_envelope_txs_insufficient_utxos() {
        let config = get_test_config();
        let chunks = vec![vec![0u8; 150], vec![0u8; 150], vec![0u8; 150]];
        let magic = MagicBytes::from([0xAA, 0xBB, 0xCC, 0xDD]);
        let kp = test_keypair();

        let address = config.sequencer_address.clone();
        let insufficient_utxos = vec![ListUnspentItem {
            txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                .parse::<Txid>()
                .unwrap(),
            vout: 0,
            address: address.as_unchecked().clone(),
            script_pubkey: ScriptBuf::new(),
            amount: Amount::from_sat(1_000),
            confirmations: 100,
            spendable: true,
            solvable: true,
            label: "".to_string(),
            safe: true,
            redeem_script: None,
            descriptor: None,
            parent_descriptors: None,
        }];

        let result = build_chunked_envelope_txs(
            &config,
            &chunks,
            &magic,
            TEST_DA_BLOB_VERSION,
            &kp,
            insufficient_utxos,
        );

        assert!(result.is_err());
    }

    #[test]
    fn build_chunked_envelope_txs_rejects_p2tr_change_address() {
        let kp = test_keypair();
        let p2tr_address = Address::p2tr(
            SECP256K1,
            XOnlyPublicKey::from_keypair(&kp).0,
            None,
            Network::Regtest,
        );
        let ctx = get_writer_context();
        let config = EnvelopeConfig::new(
            ctx.btcio_params.magic_bytes,
            p2tr_address,
            Network::Regtest,
            1000,
            546,
            None,
        );
        let chunks = vec![vec![0u8; 150]];
        let magic = MagicBytes::from([0xAA, 0xBB, 0xCC, 0xDD]);

        let result = build_chunked_envelope_txs(
            &config,
            &chunks,
            &magic,
            TEST_DA_BLOB_VERSION,
            &kp,
            get_mock_utxos(),
        );

        assert!(matches!(
            result,
            Err(EnvelopeError::P2trChangeAddressUnsupported)
        ));
    }
}
