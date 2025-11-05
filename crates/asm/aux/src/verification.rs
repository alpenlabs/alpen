use bitcoin::{Transaction, Txid};
use strata_asm_common::{
    AsmLogClaim, AsmLogEntry, AsmLogOracle, AuxResponses, CompactMmr64, L1TxClaim, L1TxIndex,
    L1TxOracle, L1TxProofBundle, Mmr64, SubprotocolId, VerifiedAuxData, VerifiedAuxInput,
};

/// Verifies a set of auxiliary responses against the global history MMR and returns the verified
/// claims. Subprotocols consume the resulting [`VerifiedAuxInput`], avoiding duplicated proof
/// checks.
pub fn verify_aux_input(
    responses: &AuxResponses,
    mmr_state: &CompactMmr64,
    subprotocol: SubprotocolId,
) -> Result<VerifiedAuxInput, AuxVerificationError> {
    if responses.data.is_empty() {
        return Ok(VerifiedAuxInput::default());
    }

    let mmr = Mmr64::from(mmr_state.clone());
    let mut verified = VerifiedAuxInput::default();
    for (tx_index, batch) in &responses.data {
        let mut verified_batch = VerifiedAuxData::default();

        for log_oracle in &batch.asm_logs {
            let verified_log = verify_asm_log_oracle(log_oracle, &mmr, *tx_index, subprotocol)?;

            verified_batch.asm_logs.push(verified_log);
        }

        for l1_oracle in &batch.l1_txs {
            verify_l1_tx_oracle(l1_oracle, *tx_index, subprotocol)?;

            verified_batch.l1_txs.push(L1TxOracle {
                tx: l1_oracle.claim.clone(),
            });
        }

        if !verified_batch.is_empty() {
            verified.data.insert(*tx_index, verified_batch);
        }
    }

    Ok(verified)
}

fn verify_asm_log_oracle(
    oracle: &AsmLogClaim,
    mmr: &Mmr64,
    tx_index: L1TxIndex,
    subprotocol: SubprotocolId,
) -> Result<AsmLogOracle, AuxVerificationError> {
    // Compute the MMR leaf from the manifest hash, matching how STF constructs it
    let manifest_hash: [u8; 32] = oracle.claim.compute_hash();
    if !mmr.verify(&oracle.proof, &manifest_hash) {
        return Err(AuxVerificationError::InvalidAsmLogProof {
            tx_index,
            subprotocol,
        });
    }

    Ok(AsmLogOracle {
        block_hash: *oracle.claim.block_root(),
        logs: oracle
            .claim
            .logs()
            .iter()
            .map(|log| AsmLogEntry::from_raw(log.as_bytes().to_vec()))
            .collect(),
    })
}

fn verify_l1_tx_oracle(
    oracle: &L1TxClaim,
    tx_index: L1TxIndex,
    subprotocol: SubprotocolId,
) -> Result<(), AuxVerificationError> {
    // Deserialize the transaction from raw bytes
    let tx = bitcoin::consensus::deserialize::<Transaction>(&oracle.claim).map_err(|err| {
        AuxVerificationError::InvalidTransaction {
            tx_index,
            subprotocol,
            reason: err.to_string(),
        }
    })?;
    let actual_txid = tx.compute_txid();

    match &oracle.proof {
        L1TxProofBundle::TxidOnly { expected_txid } => {
            let expected_from_proof: Txid = Txid::from(*expected_txid);
            if actual_txid != expected_from_proof {
                return Err(AuxVerificationError::TxidMismatch {
                    tx_index,
                    expected: expected_from_proof,
                    got: actual_txid,
                    subprotocol,
                });
            }
        }
    }

    Ok(())
}

/// Errors that can occur while verifying auxiliary input data.
#[derive(Debug)]
// todo remove subprotocol from here we having access to it in the stage and we can using it for
// logging instead passing it here
pub enum AuxVerificationError {
    InvalidAsmLogProof {
        tx_index: L1TxIndex,
        subprotocol: SubprotocolId,
    },
    InvalidTransaction {
        tx_index: L1TxIndex,
        subprotocol: SubprotocolId,
        reason: String,
    },
    TxidMismatch {
        tx_index: L1TxIndex,
        expected: Txid,
        got: Txid,
        subprotocol: SubprotocolId,
    },
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        Amount, ScriptBuf, Transaction as BtcTransaction, TxIn, TxOut,
        absolute::LockTime,
        hashes::{Hash, sha256d},
        transaction::Version,
    };
    use strata_asm_common::{
        AsmLogClaim, AsmManifest, AuxResponseBatch, AuxResponses, HEADER_MMR_CAP_LOG2, L1TxClaim,
        LogMmrProof, Mmr64, empty_history_mmr,
    };
    use strata_identifiers::Buf32;

    use super::*;

    const TEST_SUBPROTO: SubprotocolId = 42;

    fn build_raw_tx() -> BtcTransaction {
        BtcTransaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn::default()],
            output: vec![TxOut {
                value: Amount::from_sat(1),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn verify_l1_tx_oracle_txid_only_success() {
        let raw_tx = build_raw_tx();
        let txid = raw_tx.compute_txid();

        // Serialize the transaction to bytes
        let tx_bytes = bitcoin::consensus::serialize(&raw_tx);

        let mut responses = AuxResponses::default();
        responses.data.insert(
            7,
            AuxResponseBatch {
                asm_logs: vec![],
                l1_txs: vec![L1TxClaim {
                    claim: tx_bytes.clone(),
                    proof: L1TxProofBundle::TxidOnly {
                        expected_txid: Buf32::from(txid),
                    },
                }],
            },
        );

        let verified = verify_aux_input(&responses, &empty_history_mmr(), TEST_SUBPROTO).unwrap();
        let batch = verified
            .data
            .get(&7)
            .expect("expected entry for tx index 7");
        assert_eq!(batch.l1_txs.len(), 1);
        assert_eq!(batch.l1_txs[0].tx, tx_bytes);
    }

    #[test]
    fn verify_l1_tx_oracle_txid_mismatch() {
        let raw_tx = build_raw_tx();
        let tx_bytes = bitcoin::consensus::serialize(&raw_tx);

        let mut responses = AuxResponses::default();
        responses.data.insert(
            3,
            AuxResponseBatch {
                asm_logs: vec![],
                l1_txs: vec![L1TxClaim {
                    claim: tx_bytes,
                    proof: L1TxProofBundle::TxidOnly {
                        expected_txid: Buf32::from(txid_from_byte(1)),
                    },
                }],
            },
        );

        let err = verify_aux_input(&responses, &empty_history_mmr(), TEST_SUBPROTO).unwrap_err();
        assert!(matches!(err, AuxVerificationError::TxidMismatch { .. }));
    }

    #[test]
    fn verify_aux_input_accepts_valid_asm_log_proof() {
        let mut mmr = Mmr64::new(HEADER_MMR_CAP_LOG2);
        let block_root = Buf32::from([0xAB; 32]).into();
        let wtx_root = Buf32::from([0xCD; 32]);
        let logs = vec![AsmLogEntry::from_raw(vec![0x01, 0x02, 0x03])];

        // Create the manifest and compute its hash (this is what goes in the MMR)
        let manifest = AsmManifest::new(block_root, wtx_root, logs.clone());
        let manifest_hash: [u8; 32] = manifest.compute_hash();

        let mut proof_list: Vec<LogMmrProof> = Vec::new();
        let proof = mmr
            .add_leaf_updating_proof_list(manifest_hash, proof_list.as_mut_slice())
            .expect("proof generation succeeds");

        let compact_mmr = mmr.into();
        let tx_index = 5;

        let mut responses = AuxResponses::default();
        responses.data.insert(
            tx_index,
            AuxResponseBatch {
                asm_logs: vec![AsmLogClaim {
                    claim: manifest.clone(),
                    proof,
                }],
                l1_txs: vec![],
            },
        );

        let verified = verify_aux_input(&responses, &compact_mmr, TEST_SUBPROTO).unwrap();
        let batch = verified
            .data
            .get(&tx_index)
            .expect("verified batch for tx index");

        assert_eq!(batch.asm_logs.len(), 1);
        assert_eq!(batch.asm_logs[0].block_hash, block_root);
        assert_eq!(batch.asm_logs[0].logs.len(), 1);
        assert_eq!(batch.asm_logs[0].logs[0].as_bytes(), &[0x01, 0x02, 0x03]);
    }

    fn txid_from_byte(byte: u8) -> Txid {
        Txid::from_raw_hash(sha256d::Hash::from_byte_array([byte; 32]))
    }
}
