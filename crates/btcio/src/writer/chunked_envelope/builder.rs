//! Builds chunked envelope transactions: one commit tx with N P2TR outputs,
//! each spent by an independent reveal tx carrying opaque witness data.

use core::slice;

use anyhow::anyhow;
use bitcoin::{
    absolute::LockTime,
    blockdata::script,
    hashes::Hash,
    key::UntweakedKeypair,
    opcodes::all::OP_RETURN,
    secp256k1::{XOnlyPublicKey, SECP256K1},
    taproot::{LeafVersion, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
};
use bitcoind_async_client::corepc_types::model::ListUnspentItem;
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;
use strata_primitives::buf::Buf32;

use crate::writer::builder::{
    calculate_commit_output_value, choose_utxos, generate_key_pair, get_size,
    sign_reveal_transaction, EnvelopeConfig, EnvelopeError, BITCOIN_DUST_LIMIT,
};

/// Intermediate state for each reveal before tx construction.
struct RevealArtifact {
    key_pair: UntweakedKeypair,
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

/// Builds a chunked envelope from opaque chunk payloads.
///
/// Creates one commit tx funding N P2TR outputs and N reveal txs spending
/// them. Each reveal's OP_RETURN carries `magic_bytes ++ prev_wtxid` to form
/// a sequential chain: reveal 0 references `prev_tail_wtxid`, reveal 1
/// references reveal 0's wtxid, etc.
pub(crate) fn build_chunked_envelope_txs(
    config: &EnvelopeConfig,
    chunks: &[Vec<u8>],
    magic_bytes: &[u8; 4],
    prev_tail_wtxid: &Buf32,
    utxos: Vec<ListUnspentItem>,
) -> Result<ChunkedEnvelopeTxs, EnvelopeError> {
    assert!(!chunks.is_empty(), "chunks must not be empty");

    // All tag scripts have the same shape (OP_RETURN + 4 + 32 bytes), so we
    // use a single representative for commit value estimation.
    let tag_template = build_linking_tag(magic_bytes, prev_tail_wtxid);

    let mut artifacts = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let key_pair = generate_key_pair()?;
        let public_key = XOnlyPublicKey::from_keypair(&key_pair).0;

        let reveal_script = EnvelopeScriptBuilder::with_pubkey(&public_key.serialize())?
            .add_envelopes(slice::from_ref(chunk))?
            .build()?;

        let spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())?
            .finalize(SECP256K1, public_key)
            .map_err(|_| anyhow!("could not finalize taproot spend info"))?;

        let commit_value = calculate_commit_output_value(
            &config.sequencer_address,
            config.reveal_amount,
            config.fee_rate,
            &reveal_script,
            &tag_template,
            &spend_info,
        );

        artifacts.push(RevealArtifact {
            key_pair,
            reveal_script,
            spend_info,
            commit_value,
        });
    }

    let commit_tx = build_multi_output_commit(config, &artifacts, utxos)?;
    let commit_txid = commit_tx.compute_txid();

    // Build reveals sequentially — each one's OP_RETURN references the
    // previous reveal's wtxid.
    let mut wtxid = *prev_tail_wtxid;
    let mut reveal_txs = Vec::with_capacity(artifacts.len());

    for (vout, artifact) in artifacts.iter().enumerate() {
        let tag_script = build_linking_tag(magic_bytes, &wtxid);
        let commit_output = &commit_tx.output[vout];

        let control_block = artifact
            .spend_info
            .control_block(&(artifact.reveal_script.clone(), LeafVersion::TapScript))
            .ok_or_else(|| anyhow!("cannot create control block for reveal {vout}"))?;

        // Verify the commit output covers reveal fees + dust.
        let reveal_vsize = get_size(
            &[make_txin(commit_txid, vout as u32)],
            &[
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: tag_script.clone(),
                },
                TxOut {
                    value: Amount::from_sat(config.reveal_amount),
                    script_pubkey: config.sequencer_address.script_pubkey(),
                },
            ],
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
            input: vec![make_txin(commit_txid, vout as u32)],
            output: vec![
                TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: tag_script,
                },
                TxOut {
                    value: Amount::from_sat(config.reveal_amount),
                    script_pubkey: config.sequencer_address.script_pubkey(),
                },
            ],
        };

        sign_reveal_transaction(
            &mut reveal_tx,
            commit_output,
            &artifact.reveal_script,
            &artifact.spend_info,
            &artifact.key_pair,
        )?;

        wtxid = reveal_tx.compute_wtxid().as_byte_array().into();
        reveal_txs.push(reveal_tx);
    }

    Ok(ChunkedEnvelopeTxs {
        commit_tx,
        reveal_txs,
    })
}

/// `OP_RETURN <magic_bytes(4)> <prev_wtxid(32)>`.
fn build_linking_tag(magic_bytes: &[u8; 4], prev_wtxid: &Buf32) -> ScriptBuf {
    script::Builder::new()
        .push_opcode(OP_RETURN)
        .push_slice(magic_bytes)
        .push_slice(prev_wtxid.as_ref())
        .into_script()
}

/// Builds the commit tx with one P2TR output per reveal plus change.
fn build_multi_output_commit(
    config: &EnvelopeConfig,
    artifacts: &[RevealArtifact],
    utxos: Vec<ListUnspentItem>,
) -> Result<Transaction, EnvelopeError> {
    let spendable: Vec<ListUnspentItem> = utxos
        .into_iter()
        .filter(|u| u.spendable && u.solvable && u.amount.to_sat() > BITCOIN_DUST_LIMIT as i64)
        .collect();

    let reveal_outputs: Vec<TxOut> = artifacts
        .iter()
        .map(|a| {
            let pk = XOnlyPublicKey::from_keypair(&a.key_pair).0;
            let addr = Address::p2tr(SECP256K1, pk, a.spend_info.merkle_root(), config.network);
            TxOut {
                value: Amount::from_sat(a.commit_value),
                script_pubkey: addr.script_pubkey(),
            }
        })
        .collect();

    let total_output: u64 = artifacts.iter().map(|a| a.commit_value).sum();

    // Iterative fee estimation (same approach as the single-reveal builder).
    let mut last_size = get_size(
        &[make_txin(bitcoin::Txid::all_zeros(), 0)],
        &reveal_outputs,
        None,
        None,
    );

    loop {
        let fee = (last_size as u64) * config.fee_rate;
        let needed = total_output + fee;
        let (chosen, sum) = choose_utxos(&spendable, needed)?;

        let inputs: Vec<TxIn> = chosen.iter().map(|u| make_txin(u.txid, u.vout)).collect();

        let mut outputs = reveal_outputs.clone();
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
    use bitcoin::{Network, ScriptBuf, SignedAmount, Txid};
    use bitcoind_async_client::corepc_types::model::ListUnspentItem;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::test_utils::test_context::get_writer_context;

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
        )
    }

    #[test]
    fn test_build_chunked_envelope_txs_single_chunk() {
        let config = get_test_config();
        let utxos = get_mock_utxos();
        let chunks = vec![vec![0u8; 150]];
        let magic = [0xAA, 0xBB, 0xCC, 0xDD];
        let prev_wtxid = Buf32::zero();

        let result =
            build_chunked_envelope_txs(&config, &chunks, &magic, &prev_wtxid, utxos).unwrap();

        // 1 chunk → commit has 1 P2TR output + change
        assert_eq!(
            result.commit_tx.output.len(),
            2,
            "commit should have P2TR + change"
        );
        assert_eq!(result.reveal_txs.len(), 1, "should have 1 reveal");

        // Reveal spends commit output 0
        let reveal = &result.reveal_txs[0];
        assert_eq!(
            reveal.input[0].previous_output.txid,
            result.commit_tx.compute_txid(),
            "reveal should spend commit"
        );
        assert_eq!(reveal.input[0].previous_output.vout, 0);

        // Reveal has 2 outputs: OP_RETURN tag + value
        assert_eq!(reveal.output.len(), 2);
        assert_eq!(
            reveal.output[0].value.to_sat(),
            0,
            "tag output should be 0 value"
        );
        assert_eq!(
            reveal.output[1].script_pubkey,
            config.sequencer_address.script_pubkey(),
        );
    }

    #[test]
    fn test_build_chunked_envelope_txs_multiple_chunks() {
        let config = get_test_config();
        let utxos = get_mock_utxos();
        let chunks = vec![vec![1u8; 150], vec![2u8; 150], vec![3u8; 150]];
        let magic = [0xAA, 0xBB, 0xCC, 0xDD];
        let prev_wtxid = Buf32::zero();

        let result =
            build_chunked_envelope_txs(&config, &chunks, &magic, &prev_wtxid, utxos).unwrap();

        // 3 chunks → commit has 3 P2TR outputs + change
        assert_eq!(
            result.commit_tx.output.len(),
            4,
            "commit should have 3 P2TR + change"
        );
        assert_eq!(result.reveal_txs.len(), 3, "should have 3 reveals");

        let commit_txid = result.commit_tx.compute_txid();
        for (i, reveal) in result.reveal_txs.iter().enumerate() {
            assert_eq!(
                reveal.input[0].previous_output.txid, commit_txid,
                "reveal {i} should spend commit"
            );
            assert_eq!(
                reveal.input[0].previous_output.vout, i as u32,
                "reveal {i} should spend output {i}"
            );
        }
    }

    #[test]
    fn test_wtxid_chain_linking() {
        let config = get_test_config();
        let utxos = get_mock_utxos();
        let chunks = vec![vec![1u8; 150], vec![2u8; 150], vec![3u8; 150]];
        let magic = [0xAA, 0xBB, 0xCC, 0xDD];
        let prev_wtxid = Buf32::zero();

        let result =
            build_chunked_envelope_txs(&config, &chunks, &magic, &prev_wtxid, utxos).unwrap();

        // Each reveal's OP_RETURN tag should reference the previous reveal's wtxid.
        let expected_tag_0 = build_linking_tag(&magic, &prev_wtxid);
        assert_eq!(
            result.reveal_txs[0].output[0].script_pubkey, expected_tag_0,
            "reveal 0 should reference prev_tail_wtxid"
        );

        let wtxid_0: Buf32 = result.reveal_txs[0].compute_wtxid().as_byte_array().into();
        let expected_tag_1 = build_linking_tag(&magic, &wtxid_0);
        assert_eq!(
            result.reveal_txs[1].output[0].script_pubkey, expected_tag_1,
            "reveal 1 should reference reveal 0's wtxid"
        );

        let wtxid_1: Buf32 = result.reveal_txs[1].compute_wtxid().as_byte_array().into();
        let expected_tag_2 = build_linking_tag(&magic, &wtxid_1);
        assert_eq!(
            result.reveal_txs[2].output[0].script_pubkey, expected_tag_2,
            "reveal 2 should reference reveal 1's wtxid"
        );
    }

    #[test]
    fn test_build_chunked_envelope_txs_insufficient_utxos() {
        let config = get_test_config();
        let chunks = vec![vec![0u8; 150], vec![0u8; 150], vec![0u8; 150]];
        let magic = [0xAA, 0xBB, 0xCC, 0xDD];
        let prev_wtxid = Buf32::zero();

        let address = config.sequencer_address.clone();
        let insufficient_utxos = vec![ListUnspentItem {
            txid: "4cfbec13cf1510545f285cceceb6229bd7b6a918a8f6eba1dbee64d26226a3b7"
                .parse::<Txid>()
                .unwrap(),
            vout: 0,
            address: address.as_unchecked().clone(),
            script_pubkey: ScriptBuf::new(),
            amount: SignedAmount::from_sat(1000), // far too little for 3 reveals
            confirmations: 100,
            spendable: true,
            solvable: true,
            label: "".to_string(),
            safe: true,
            redeem_script: None,
            descriptor: None,
            parent_descriptors: None,
        }];

        let result =
            build_chunked_envelope_txs(&config, &chunks, &magic, &prev_wtxid, insufficient_utxos);

        assert!(result.is_err(), "should fail with insufficient UTXOs");
        match result {
            Err(EnvelopeError::NotEnoughUtxos(needed, have)) => {
                assert!(
                    needed > have,
                    "needed ({needed}) should exceed have ({have})"
                );
            }
            other => panic!("expected NotEnoughUtxos error, got: {other:?}"),
        }
    }

    #[test]
    fn test_build_linking_tag_structure() {
        let magic = [0xDE, 0xAD, 0xBE, 0xEF];
        let wtxid = Buf32::from([0x42; 32]);
        let tag = build_linking_tag(&magic, &wtxid);

        // OP_RETURN (0x6a) + push4 (0x04) + 4 bytes + push32 (0x20) + 32 bytes
        let bytes = tag.as_bytes();
        assert_eq!(bytes[0], 0x6a, "should start with OP_RETURN");
        assert_eq!(bytes[1], 4, "push 4 bytes for magic");
        assert_eq!(&bytes[2..6], &magic);
        assert_eq!(bytes[6], 32, "push 32 bytes for wtxid");
        assert_eq!(&bytes[7..39], wtxid.as_ref());
    }
}
