//! Transaction construction for chunked envelope publication.

use std::cmp::Reverse;

use bitcoin::{
    absolute::LockTime,
    hashes::Hash,
    key::UntweakedKeypair,
    secp256k1::{Message, SECP256K1},
    sighash::{Prevouts, SighashCache, TapSighashType},
    taproot::{ControlBlock, LeafVersion, TapLeafHash, TaprootBuilder, TaprootSpendInfo},
    transaction::Version,
    Address, Amount, Network, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness,
    Wtxid, XOnlyPublicKey,
};
use bitcoind_async_client::corepc_types::model::ListUnspentItem;
use rand::{rngs::OsRng, RngCore};
use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;

use super::{
    op_return::RawOpReturnBuilder,
    types::{ChunkedEnvelopeHeader, ChunkedPayloadIntent},
    ChunkedEnvelopeError,
};

/// Bitcoin dust limit for P2TR outputs.
const DUST_LIMIT: u64 = 546;

/// DA type byte for OP_RETURN.
const DA_TYPE_BYTE: u8 = 0x01;

/// Configuration for building chunked envelope transactions.
#[derive(Debug, Clone)]
pub(crate) struct ChunkedEnvelopeConfig {
    /// Sequencer address for change and reveal outputs.
    pub sequencer_address: Address,
    /// Bitcoin network.
    pub network: Network,
    /// Fee rate in sats/vByte.
    pub fee_rate: u64,
    /// Output value for each commit output (should cover reveal tx fees + dust).
    pub commit_output_value: u64,
    /// Output value for reveal recipient output (dust).
    pub reveal_output_value: u64,
}

impl ChunkedEnvelopeConfig {
    /// Creates a new config with reasonable defaults.
    pub(crate) fn new(sequencer_address: Address, network: Network, fee_rate: u64) -> Self {
        // Each commit output needs to cover:
        // - Reveal tx fee (~150 vbytes * fee_rate for small reveals, more for large)
        // - Dust output (546 sats)
        // Conservative estimate for 330KB reveal: ~350 vbytes worst case
        let reveal_fee_estimate = 400 * fee_rate;
        let commit_output_value = reveal_fee_estimate + DUST_LIMIT;

        Self {
            sequencer_address,
            network,
            fee_rate,
            commit_output_value,
            reveal_output_value: DUST_LIMIT,
        }
    }
}

/// Data needed to construct reveal transactions.
#[derive(Debug)]
pub(crate) struct RevealData {
    /// Ephemeral keypair for this chunk.
    pub(crate) keypair: UntweakedKeypair,
    /// Public key (stored for debugging and future extensibility).
    #[expect(
        dead_code,
        reason = "kept for debugging via Debug trait and potential future use"
    )]
    pub(crate) pubkey: XOnlyPublicKey,
    /// Reveal script.
    pub(crate) reveal_script: ScriptBuf,
    /// Taproot spend info (stored for debugging and future extensibility).
    #[expect(
        dead_code,
        reason = "kept for debugging via Debug trait and potential future use"
    )]
    pub(crate) taproot_info: TaprootSpendInfo,
    /// Taproot address for commit output.
    pub(crate) address: Address,
    /// Control block for script-path spend.
    pub(crate) control_block: ControlBlock,
}

/// Result of building all transactions for a chunked envelope.
#[derive(Debug)]
pub(crate) struct BuiltChunkedEnvelope {
    /// Commit transaction (unsigned, needs signing by wallet).
    pub(crate) commit_tx: Transaction,
    /// Reveal transactions (signed with ephemeral keys).
    pub(crate) reveal_txs: Vec<Transaction>,
}

/// Builds all transactions for a chunked envelope.
///
/// This is the main entry point for transaction construction.
pub(crate) fn build_chunked_envelope_txs(
    intent: &ChunkedPayloadIntent,
    utxos: Vec<ListUnspentItem>,
    config: &ChunkedEnvelopeConfig,
) -> Result<BuiltChunkedEnvelope, ChunkedEnvelopeError> {
    let payload_hash = intent.compute_payload_hash();
    let chunks = intent.split_into_chunks();
    let total_chunks = chunks.len() as u16;

    // 1. Build reveal data for each chunk (scripts, addresses, keypairs)
    let reveal_data: Vec<RevealData> = chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            build_reveal_data(
                intent.op_return_tag(),
                &payload_hash.0,
                i as u16,
                total_chunks,
                chunk,
                config.network,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // 2. Compute total required funds
    let total_commit_outputs = total_chunks as u64 * config.commit_output_value;
    let estimated_commit_fee = estimate_commit_fee(total_chunks as usize, config.fee_rate);
    let total_required = total_commit_outputs + estimated_commit_fee;

    // Check available balance
    let available: u64 = utxos
        .iter()
        .filter(|u| u.spendable && u.solvable)
        .map(|u| u.amount.to_sat() as u64)
        .sum();

    if available < total_required {
        return Err(ChunkedEnvelopeError::InsufficientUtxos {
            required: total_required,
            available,
        });
    }

    // 3. Build batched commit transaction
    let commit_tx = build_batched_commit_tx(
        &utxos,
        &reveal_data,
        &config.sequencer_address,
        config.commit_output_value,
        config.fee_rate,
    )?;

    let commit_txid = commit_tx.compute_txid();

    // 4. Build reveal transactions sequentially (for wtxid linking)
    let mut reveal_txs: Vec<Transaction> = Vec::with_capacity(total_chunks as usize);
    let mut reveal_wtxids: Vec<Wtxid> = Vec::with_capacity(total_chunks as usize);

    for (i, data) in reveal_data.iter().enumerate() {
        // Determine prev_chunk_wtxid
        let prev_wtxid: [u8; 32] = if i == 0 {
            // First chunk: use prev_tail_wtxid or zeros
            intent
                .prev_tail_wtxid()
                .map(|w| w.to_byte_array())
                .unwrap_or([0u8; 32])
        } else {
            // Subsequent chunks: use previous reveal's wtxid
            reveal_wtxids[i - 1].to_byte_array()
        };

        // Build and sign reveal tx
        let reveal_tx = build_reveal_tx(
            commit_txid,
            i as u32,
            config.commit_output_value,
            data,
            intent.op_return_tag(),
            &prev_wtxid,
            &config.sequencer_address,
            config.reveal_output_value,
        )?;

        let wtxid = reveal_tx.compute_wtxid();
        reveal_txs.push(reveal_tx);
        reveal_wtxids.push(wtxid);
    }

    Ok(BuiltChunkedEnvelope {
        commit_tx,
        reveal_txs,
    })
}

/// Builds reveal data (script, address, keypair) for a single chunk.
fn build_reveal_data(
    op_return_tag: [u8; 4],
    payload_hash: &[u8; 32],
    chunk_index: u16,
    total_chunks: u16,
    chunk_payload: &[u8],
    network: Network,
) -> Result<RevealData, ChunkedEnvelopeError> {
    // Generate ephemeral keypair
    let keypair = generate_keypair()?;
    let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;

    // Build chunk header
    let header = ChunkedEnvelopeHeader::new(*payload_hash, chunk_index, total_chunks)
        .map_err(ChunkedEnvelopeError::InvalidHeader)?;

    // Build envelope data: tag || header || payload
    let mut envelope_data = Vec::with_capacity(4 + ChunkedEnvelopeHeader::SIZE + chunk_payload.len());
    envelope_data.extend_from_slice(&op_return_tag);
    envelope_data.extend_from_slice(&header.serialize());
    envelope_data.extend_from_slice(chunk_payload);

    // Build reveal script using EnvelopeScriptBuilder
    // add_envelopes expects an iterator of byte slices, so wrap our single envelope
    let reveal_script = EnvelopeScriptBuilder::with_pubkey(&pubkey.serialize())
        .map_err(|e| ChunkedEnvelopeError::TxBuild(e.to_string()))?
        .add_envelopes(&[envelope_data.as_slice()])
        .map_err(|e| ChunkedEnvelopeError::TxBuild(e.to_string()))?
        .build()
        .map_err(|e| ChunkedEnvelopeError::TxBuild(e.to_string()))?;

    // Build taproot spend info
    let taproot_info = TaprootBuilder::new()
        .add_leaf(0, reveal_script.clone())
        .map_err(|e| ChunkedEnvelopeError::TxBuild(e.to_string()))?
        .finalize(SECP256K1, pubkey)
        .map_err(|_| ChunkedEnvelopeError::TxBuild("failed to finalize taproot".to_string()))?;

    // Derive address
    let address = Address::p2tr(SECP256K1, pubkey, taproot_info.merkle_root(), network);

    // Get control block
    let control_block = taproot_info
        .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
        .ok_or_else(|| ChunkedEnvelopeError::TxBuild("failed to create control block".to_string()))?;

    Ok(RevealData {
        keypair,
        pubkey,
        reveal_script,
        taproot_info,
        address,
        control_block,
    })
}

/// Builds the batched commit transaction with N outputs.
fn build_batched_commit_tx(
    utxos: &[ListUnspentItem],
    reveal_data: &[RevealData],
    change_address: &Address,
    output_value_per_chunk: u64,
    fee_rate: u64,
) -> Result<Transaction, ChunkedEnvelopeError> {
    let num_chunks = reveal_data.len();

    // Filter spendable UTXOs
    let spendable_utxos: Vec<&ListUnspentItem> = utxos
        .iter()
        .filter(|u| u.spendable && u.solvable && u.amount.to_sat() > DUST_LIMIT as i64)
        .collect();

    // Build outputs (one per chunk + optional change)
    let mut outputs: Vec<TxOut> = reveal_data
        .iter()
        .map(|data| TxOut {
            value: Amount::from_sat(output_value_per_chunk),
            script_pubkey: data.address.script_pubkey(),
        })
        .collect();

    // Calculate total output value
    let total_output = num_chunks as u64 * output_value_per_chunk;

    // Select UTXOs
    let (selected_utxos, total_input) = select_utxos(&spendable_utxos, total_output, fee_rate)?;

    // Build inputs
    let inputs: Vec<TxIn> = selected_utxos
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
        .collect();

    // Estimate fee
    let estimated_size = estimate_tx_vsize(inputs.len(), outputs.len() + 1); // +1 for potential change
    let fee = estimated_size as u64 * fee_rate;

    // Add change output if needed
    let change = total_input.saturating_sub(total_output + fee);
    if change > DUST_LIMIT {
        outputs.push(TxOut {
            value: Amount::from_sat(change),
            script_pubkey: change_address.script_pubkey(),
        });
    }

    Ok(Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: inputs,
        output: outputs,
    })
}

/// Builds and signs a reveal transaction.
#[expect(
    clippy::too_many_arguments,
    reason = "internal helper with cohesive set of parameters"
)]
fn build_reveal_tx(
    commit_txid: bitcoin::Txid,
    vout: u32,
    input_value: u64,
    reveal_data: &RevealData,
    op_return_tag: [u8; 4],
    prev_chunk_wtxid: &[u8; 32],
    recipient: &Address,
    output_value: u64,
) -> Result<Transaction, ChunkedEnvelopeError> {
    // Build OP_RETURN script
    let op_return_script = RawOpReturnBuilder::with_tag(op_return_tag)
        .push_byte(DA_TYPE_BYTE)
        .push_bytes(prev_chunk_wtxid)
        .build()
        .map_err(|e| ChunkedEnvelopeError::TxBuild(e.to_string()))?;

    // Build outputs
    let outputs = vec![
        TxOut {
            value: Amount::ZERO,
            script_pubkey: op_return_script,
        },
        TxOut {
            value: Amount::from_sat(output_value),
            script_pubkey: recipient.script_pubkey(),
        },
    ];

    // Build input
    let input = TxIn {
        previous_output: OutPoint {
            txid: commit_txid,
            vout,
        },
        script_sig: ScriptBuf::new(),
        witness: Witness::new(),
        sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
    };

    let mut tx = Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input: vec![input],
        output: outputs,
    };

    // Sign the reveal transaction
    sign_reveal_tx(&mut tx, input_value, reveal_data)?;

    Ok(tx)
}

/// Signs a reveal transaction using script-path spend.
fn sign_reveal_tx(
    tx: &mut Transaction,
    input_value: u64,
    reveal_data: &RevealData,
) -> Result<(), ChunkedEnvelopeError> {
    let prevout = TxOut {
        value: Amount::from_sat(input_value),
        script_pubkey: reveal_data.address.script_pubkey(),
    };

    // Compute sighash
    let mut sighash_cache = SighashCache::new(&*tx);
    let sighash = sighash_cache
        .taproot_script_spend_signature_hash(
            0,
            &Prevouts::All(&[prevout]),
            TapLeafHash::from_script(
                &reveal_data.reveal_script,
                LeafVersion::TapScript,
            ),
            TapSighashType::Default,
        )
        .map_err(|e| ChunkedEnvelopeError::TxBuild(format!("sighash error: {e}")))?;

    // Sign
    let msg = Message::from_digest(sighash.to_byte_array());
    let signature = SECP256K1.sign_schnorr(&msg, &reveal_data.keypair);

    // Build witness: [signature, script, control_block]
    let mut witness = Witness::new();
    witness.push(signature.as_ref());
    witness.push(reveal_data.reveal_script.as_bytes());
    witness.push(reveal_data.control_block.serialize());

    tx.input[0].witness = witness;

    Ok(())
}

/// Generates an ephemeral keypair.
fn generate_keypair() -> Result<UntweakedKeypair, ChunkedEnvelopeError> {
    let mut rand_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut rand_bytes);
    UntweakedKeypair::from_seckey_slice(SECP256K1, &rand_bytes)
        .map_err(|e| ChunkedEnvelopeError::Signing(e.to_string()))
}

/// Selects UTXOs to cover the required amount.
fn select_utxos(
    utxos: &[&ListUnspentItem],
    required: u64,
    fee_rate: u64,
) -> Result<(Vec<ListUnspentItem>, u64), ChunkedEnvelopeError> {
    // Simple greedy selection - largest first
    let mut sorted: Vec<_> = utxos.iter().collect();
    sorted.sort_by_key(|u| Reverse(u.amount.to_sat()));

    let mut selected = Vec::new();
    let mut total: u64 = 0;

    // Add buffer for input fees
    let per_input_fee = 68 * fee_rate; // ~68 vbytes per P2WPKH input

    for utxo in sorted {
        selected.push((*utxo).clone());
        total += utxo.amount.to_sat() as u64;

        let input_fees = selected.len() as u64 * per_input_fee;
        if total >= required + input_fees {
            return Ok((selected, total));
        }
    }

    let available = total;
    Err(ChunkedEnvelopeError::InsufficientUtxos {
        required,
        available,
    })
}

/// Estimates commit transaction fee.
fn estimate_commit_fee(num_outputs: usize, fee_rate: u64) -> u64 {
    // Rough estimate: 1 input, N outputs
    // Base: 10 vbytes + input: 68 vbytes + outputs: 43 vbytes each
    let vsize = 10 + 68 + (num_outputs + 1) * 43; // +1 for change output
    vsize as u64 * fee_rate
}

/// Estimates transaction virtual size.
fn estimate_tx_vsize(num_inputs: usize, num_outputs: usize) -> usize {
    // Rough estimate for P2WPKH inputs, P2TR outputs
    // Base: 10 vbytes
    // Per input: 68 vbytes (P2WPKH)
    // Per output: 43 vbytes (P2TR)
    10 + num_inputs * 68 + num_outputs * 43
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_header_in_envelope() {
        let payload_hash = [0x42u8; 32];
        let header = ChunkedEnvelopeHeader::new(payload_hash, 0, 1).unwrap();
        let serialized = header.serialize();
        assert_eq!(serialized.len(), ChunkedEnvelopeHeader::SIZE);
        assert_eq!(serialized[0], 0); // version
        assert_eq!(&serialized[1..33], &payload_hash);
    }

    #[test]
    fn test_op_return_size() {
        let prev_wtxid = [0u8; 32];
        let script = RawOpReturnBuilder::with_tag(*b"EEDA")
            .push_byte(DA_TYPE_BYTE)
            .push_bytes(&prev_wtxid)
            .build()
            .unwrap();

        // OP_RETURN (1) + PUSH opcode (1) + data (37) = 39 bytes
        assert_eq!(script.len(), 39);
    }
}
