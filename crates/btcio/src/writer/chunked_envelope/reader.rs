//! Inverse of the chunk-encoding side: given a reveal tx produced by
//! [`super::builder::build_chunked_envelope_txs`], lift the
//! `chunk_header || chunk_payload` bytes that
//! `alpen_ee_common::decode_da_chunk` consumes.
//!
//! Used by the EE outer (acct) proof's host-side input assembly to plumb
//! DA witness data into the prover.
//
// FIXME(#1751): the chunked-envelope wire format is being redesigned in
// alpenlabs/alpen#1751 ("EE DA Chunked Envelope Redesign"). Once that
// lands, this extractor (and the matching guest-side reassembly) needs
// to be reworked:
//   - the per-chunk witness header (`version || blob_hash || chunk_index || total_chunks`) goes
//     away — chunks become raw payload bytes only;
//   - `blob_hash` moves to the commit tx's OP_RETURN, so the acct host must also fetch the commit
//     and the guest must verify it;
//   - reveal tapscript becomes `<sequencer_pk> OP_CHECKSIG OP_FALSE OP_IF <chunk> OP_ENDIF`, so the
//     guest must additionally check the schnorr signature is valid under `sequencer_pk`.
// See PR #1751 and the redesign doc for details.

use bitcoin::{ScriptBuf, Transaction};
use strata_l1_envelope_fmt::{errors::EnvelopeParseError, parser::parse_envelope_payload};

/// Errors lifting an envelope payload out of a reveal tx.
#[derive(Debug, thiserror::Error)]
pub enum ExtractRevealError {
    #[error("reveal tx has no inputs")]
    MissingInputs,
    #[error("reveal tx witness has no tapscript leaf")]
    MissingLeafScript,
    #[error("envelope parse error: {0}")]
    EnvelopeParse(#[from] EnvelopeParseError),
}

/// Extracts the chunked-envelope payload bytes (`chunk_header ||
/// chunk_payload`) from a reveal tx's witness.
///
/// The reveal tx's first input must spend a tapscript path whose leaf
/// is a chunked-envelope reveal script of the shape
/// `<pubkey> OP_CHECKSIG OP_FALSE OP_IF <chunk_bytes> OP_ENDIF`.
pub fn extract_chunk_envelope_payload(reveal: &Transaction) -> Result<Vec<u8>, ExtractRevealError> {
    let input = reveal
        .input
        .first()
        .ok_or(ExtractRevealError::MissingInputs)?;
    let leaf = input
        .witness
        .taproot_leaf_script()
        .ok_or(ExtractRevealError::MissingLeafScript)?;
    let script: ScriptBuf = leaf.script.into();
    Ok(parse_envelope_payload(&script)?)
}

#[cfg(test)]
mod tests {
    use core::slice;

    use bitcoin::{
        absolute::LockTime,
        key::UntweakedKeypair,
        secp256k1::SECP256K1,
        taproot::{ControlBlock, LeafVersion, TaprootBuilder},
        transaction::Version,
        Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, XOnlyPublicKey,
    };
    use strata_l1_envelope_fmt::builder::EnvelopeScriptBuilder;

    use super::*;

    /// Builds a reveal-shaped tx whose first input's witness carries a
    /// single-envelope tapscript with the given payload bytes.
    fn make_fake_reveal(chunk_bytes: &[u8]) -> Transaction {
        let secret_bytes = [0x42u8; 32];
        let key_pair = UntweakedKeypair::from_seckey_slice(SECP256K1, &secret_bytes).unwrap();
        let pubkey = XOnlyPublicKey::from_keypair(&key_pair).0;

        let reveal_script = EnvelopeScriptBuilder::with_pubkey(&pubkey.serialize())
            .unwrap()
            .add_envelopes(slice::from_ref(&chunk_bytes.to_vec()))
            .unwrap()
            .build_without_min_check()
            .unwrap();

        let spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())
            .unwrap()
            .finalize(SECP256K1, pubkey)
            .unwrap();
        let control_block: ControlBlock = spend_info
            .control_block(&(reveal_script.clone(), LeafVersion::TapScript))
            .unwrap();

        let mut witness = Witness::new();
        // Stack item: dummy signature placeholder. Verification isn't
        // exercised here — `taproot_leaf_script()` only walks the
        // last-two-items convention.
        witness.push([0u8; 64]);
        witness.push(reveal_script.as_bytes());
        witness.push(control_block.serialize());

        Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness,
            }],
            output: vec![TxOut {
                value: Amount::from_sat(0),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    #[test]
    fn extracts_small_chunk_payload() {
        let chunk = vec![0xAB, 0xCD, 0xEF, 0x01, 0x23];
        let tx = make_fake_reveal(&chunk);
        let extracted = extract_chunk_envelope_payload(&tx).expect("extraction must succeed");
        assert_eq!(extracted, chunk);
    }

    #[test]
    fn extracts_chunk_spanning_multiple_pushes() {
        // > 520 bytes forces the envelope builder to split into multiple
        // pushdata segments; the parser must concatenate them back.
        let chunk: Vec<u8> = (0..2_000u32).map(|i| (i % 256) as u8).collect();
        let tx = make_fake_reveal(&chunk);
        let extracted = extract_chunk_envelope_payload(&tx).expect("extraction must succeed");
        assert_eq!(extracted, chunk);
    }

    #[test]
    fn fails_on_witness_without_leaf_script() {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint::null(),
                script_sig: ScriptBuf::new(),
                sequence: Sequence::MAX,
                witness: Witness::new(),
            }],
            output: vec![],
        };
        let err = extract_chunk_envelope_payload(&tx).unwrap_err();
        assert!(matches!(err, ExtractRevealError::MissingLeafScript));
    }

    #[test]
    fn fails_on_no_inputs() {
        let tx = Transaction {
            version: Version::TWO,
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        let err = extract_chunk_envelope_payload(&tx).unwrap_err();
        assert!(matches!(err, ExtractRevealError::MissingInputs));
    }
}
