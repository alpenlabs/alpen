#![no_main]

mod common;

use arbitrary::Arbitrary;
use bitcoin::{Amount, ScriptBuf, Transaction, TxOut, absolute::LockTime, transaction::Version};
use common::{FuzzTxInput, FuzzTxOutput, ArbitraryOpReturnScript};
use libfuzzer_sys::fuzz_target;
use strata_asm_common::TxInputRef;
use strata_asm_txs_bridge_v1::deposit::{parse_deposit_tx, DepositTxHeaderAux};
use strata_l1_txfmt::{ParseConfig, TagData};

/// Adversarial fuzzing for deposit parser
///
/// This fuzzer intentionally:
/// - Feeds malformed data to the parser (no sanitization)
/// - Decouples script encoding from parsing (tests arbitrary OP_RETURN structures)
/// - Varies transaction structure (version, locktime, inputs, witness)
/// - Tests outputs in arbitrary positions (OP_RETURN not always at index 0)
/// - Mutates aux data aggressively (bit flips, random bytes, malformed structures)
/// - Asserts basic invariants when parsing succeeds
#[derive(Arbitrary, Debug)]
struct FuzzDepositTx {
    /// Transaction structure fuzzing
    version: i32,
    lock_time_raw: u32,
    input_count: u8,          // 0-255, capped at 5
    inputs: [FuzzTxInput; 5],

    /// Output position fuzzing - OP_RETURN doesn't have to be at index 0
    op_return_position: u8,   // Where to insert the OP_RETURN
    output_count: u8,         // Total outputs (0-20)
    outputs: [FuzzTxOutput; 10],

    /// OP_RETURN script fuzzing - arbitrary bytes, not using ParseConfig encoder
    op_return_script: ArbitraryOpReturnScript,

    /// Aux data fuzzing - raw bytes fed to decoder, no validation
    /// This is the main attack surface for parse_deposit_tx
    raw_aux_data: Vec<u8>,

    /// For cases where we want to test "almost valid" inputs
    /// When true, we'll try to construct something closer to valid
    use_structured_aux: bool,
    structured_aux: DepositTxHeaderAux,
    aux_corruption_strategy: AuxCorruptionStrategy,

    /// Magic bytes for ParseConfig (can mismatch)
    producer_magic: [u8; 4],
    consumer_magic: [u8; 4],

    /// Test different parsing paths
    force_invalid_tagdata: bool,
    tagdata_subprotocol_id: u8,
    tagdata_tx_type: u8,
}

#[derive(Arbitrary, Debug, Clone)]
enum AuxCorruptionStrategy {
    None,
    Truncate(u8),           // Remove N bytes from end
    Extend(Vec<u8>),        // Append arbitrary bytes
    BitFlip(u8),            // Flip bit at position N % len
    RandomBytes(Vec<u8>),   // Replace with completely random bytes
    PartialOverwrite { offset: u8, data: Vec<u8> }, // Overwrite at offset
    ZeroPrefix(u8),         // Add N zero bytes at start
    DuplicateFields,        // Try to duplicate internal fields
}

impl FuzzDepositTx {
    fn to_bitcoin_tx(&self) -> Transaction {
        // Construct inputs - vary input count
        let input_count = self.input_count.min(5) as usize;
        let mut tx_inputs = Vec::new();
        for i in 0..input_count.max(1) {
            // At least one input required by Bitcoin
            if i < self.inputs.len() {
                tx_inputs.push(self.inputs[i].to_txin());
            } else {
                // Fallback to first input if we run out
                tx_inputs.push(self.inputs[0].to_txin());
            }
        }

        // Construct outputs - arbitrary order, OP_RETURN not necessarily at index 0
        let output_count = self.output_count.min(20) as usize;
        let op_return_pos = (self.op_return_position as usize) % (output_count + 1);

        let mut tx_outputs = Vec::new();
        for i in 0..output_count {
            if i == op_return_pos {
                // Insert OP_RETURN at this position
                tx_outputs.push(TxOut {
                    value: Amount::ZERO,
                    script_pubkey: self.op_return_script.to_script(),
                });
            }

            // Add regular output
            if i < self.outputs.len() {
                tx_outputs.push(self.outputs[i].to_txout());
            } else {
                // Create minimal output when we run out
                tx_outputs.push(TxOut {
                    value: Amount::from_sat(0),
                    script_pubkey: ScriptBuf::new(),
                });
            }
        }

        // If op_return_pos >= output_count, append it at the end
        if op_return_pos >= output_count {
            tx_outputs.push(TxOut {
                value: Amount::ZERO,
                script_pubkey: self.op_return_script.to_script(),
            });
        }

        Transaction {
            version: Version(self.version),
            lock_time: LockTime::from_consensus(self.lock_time_raw),
            input: tx_inputs,
            output: tx_outputs,
        }
    }

    fn get_aux_data(&self) -> Vec<u8> {
        if self.use_structured_aux {
            // Start with valid encoded aux data, then corrupt it
            let mut aux_data = strata_codec::encode_to_vec(&self.structured_aux)
                .unwrap_or_else(|_| {
                    // If encoding fails, use raw malformed data
                    self.raw_aux_data.clone()
                });

            // Apply corruption strategy
            match &self.aux_corruption_strategy {
                AuxCorruptionStrategy::None => aux_data,
                AuxCorruptionStrategy::Truncate(n) => {
                    let remove = (*n as usize).min(aux_data.len());
                    aux_data.truncate(aux_data.len().saturating_sub(remove));
                    aux_data
                }
                AuxCorruptionStrategy::Extend(bytes) => {
                    aux_data.extend_from_slice(bytes);
                    aux_data
                }
                AuxCorruptionStrategy::BitFlip(pos) => {
                    if !aux_data.is_empty() {
                        let idx = (*pos as usize) % aux_data.len();
                        let bit = *pos % 8;
                        aux_data[idx] ^= 1 << bit;
                    }
                    aux_data
                }
                AuxCorruptionStrategy::RandomBytes(bytes) => bytes.clone(),
                AuxCorruptionStrategy::PartialOverwrite { offset, data } => {
                    let start = (*offset as usize).min(aux_data.len());
                    for (i, byte) in data.iter().enumerate() {
                        if start + i < aux_data.len() {
                            aux_data[start + i] = *byte;
                        } else {
                            aux_data.push(*byte);
                        }
                    }
                    aux_data
                }
                AuxCorruptionStrategy::ZeroPrefix(n) => {
                    let mut result = vec![0u8; *n as usize];
                    result.extend_from_slice(&aux_data);
                    result
                }
                AuxCorruptionStrategy::DuplicateFields => {
                    // Try to duplicate the entire structure
                    let mut result = aux_data.clone();
                    result.extend_from_slice(&aux_data);
                    result
                }
            }
        } else {
            // Use completely raw arbitrary bytes
            self.raw_aux_data.clone()
        }
    }
}

fuzz_target!(|input: FuzzDepositTx| {
    let tx = input.to_bitcoin_tx();

    // Use consumer magic bytes for parsing (can differ from producer)
    let parse_config = ParseConfig::new(input.consumer_magic);

    // Try to parse the OP_RETURN script
    // CRITICAL: Don't bail on parse failure - we want to test how parse_deposit_tx
    // handles malformed TagData, not just test try_parse_tx
    let tag_data_result = parse_config.try_parse_tx(&tx);

    match tag_data_result {
        Ok(tag_data_ref) => {
            // Successfully parsed OP_RETURN - feed to parse_deposit_tx
            let tx_input = TxInputRef::new(&tx, tag_data_ref);
            let parse_result = parse_deposit_tx(&tx_input);

            // Assert basic invariants when parsing succeeds
            if let Ok(deposit_info) = parse_result {
                // Invariant: deposit output must exist at index 1
                assert!(
                    tx.output.len() > 1,
                    "Parser returned Ok but output[1] doesn't exist"
                );

                // Invariant: parsed aux data should round-trip if we re-encode it
                let re_encoded = strata_codec::encode_to_vec(deposit_info.header_aux());
                if re_encoded.is_ok() {
                    // If it encodes successfully, decoding should work
                    let re_decoded: Result<DepositTxHeaderAux, _> =
                        strata_codec::decode_buf_exact(&re_encoded.unwrap());
                    assert!(
                        re_decoded.is_ok(),
                        "Round-trip encode/decode failed for parsed aux data"
                    );
                }

                // Invariant: deposit amount should match what's in output[1]
                assert_eq!(
                    deposit_info.amt(),
                    tx.output[1].value.to_sat().into(),
                    "Parsed amount doesn't match output[1] amount"
                );

                // Invariant: deposit index should be consistent (whatever value it parsed)
                let parsed_idx = deposit_info.header_aux().deposit_idx();
                // Just check it's the same if we access it twice
                assert_eq!(parsed_idx, deposit_info.header_aux().deposit_idx());
            }
            // If parse failed, that's fine - we're just checking it doesn't panic/UB
        }
        Err(_parse_error) => {
            // try_parse_tx failed to find valid TagData
            // But we can still test parse_deposit_tx with manually constructed TagData

            if input.force_invalid_tagdata {
                // Manually construct TagData with arbitrary values
                let aux_data = input.get_aux_data();

                if let Ok(tag_data) = TagData::new(
                    input.tagdata_subprotocol_id,
                    input.tagdata_tx_type,
                    aux_data,
                ) {
                    let tx_input = TxInputRef::new(&tx, tag_data.as_ref());
                    // Feed malformed TagData to parser - this is where real bugs hide
                    let _ = parse_deposit_tx(&tx_input);
                }
            }

            // Also test direct aux data parsing by constructing TagData
            // This tests the case where try_parse_tx might be inconsistent with parse_deposit_tx
            let aux_data = input.get_aux_data();
            if let Ok(tag_data) = TagData::new(2, 1, aux_data) {
                // Use correct protocol values but malformed aux data
                let tx_input = TxInputRef::new(&tx, tag_data.as_ref());
                let _ = parse_deposit_tx(&tx_input);
            }
        }
    }
});
