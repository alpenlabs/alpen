use bitcoin::{
    consensus::deserialize,
    secp256k1::{self, schnorr},
    sighash,
};
use strata_crypto::groth16_verifier;
use strata_primitives::proof::RollupVerifyingKey;
use zkaleido::{Proof, ProofReceipt, PublicValues};

use crate::{CounterproofInputBorsh, CounterproofMode, CounterproofPublicOutput};

/// Takes witness data; returns public inputs that match given witness.
/// Fails if witness is invalid.
///
/// Verification will check if
/// 1. Witness is valid (matches program), and
/// 2. Witness matches expected public inputs.
pub(crate) fn process_counterproof(
    input: CounterproofInputBorsh,
) -> Result<CounterproofPublicOutput, &'static str> {
    let CounterproofInputBorsh {
        bridge_proof_master_key,
        deposit_index,
        bridge_proof_tx_bytes,
        bridge_proof_prevouts,
        mode,
    } = input;

    // Deserialize the transaction
    let bridge_proof_tx = deserialize::<bitcoin::Transaction>(&bridge_proof_tx_bytes)
        .map_err(|_| "failed to deserialize bridge proof transaction")?;

    // Deserialize the prevouts
    let bridge_proof_prevouts: Result<Vec<bitcoin::TxOut>, _> = bridge_proof_prevouts
        .iter()
        .map(|bytes| deserialize(bytes))
        .collect();
    let bridge_proof_prevouts =
        bridge_proof_prevouts.map_err(|_| "failed to deserialize bridge proof prevouts")?;

    // Convert master key bytes back to XOnlyPublicKey
    let bridge_proof_master_key = secp256k1::XOnlyPublicKey::from_slice(&bridge_proof_master_key)
        .map_err(|_| "invalid bridge proof master key")?;

    let mut deposit_index_le_bytes = [0; 32];
    deposit_index_le_bytes[0..4].copy_from_slice(&deposit_index.to_le_bytes());
    let deposit_index_scalar = match secp256k1::Scalar::from_le_bytes(deposit_index_le_bytes) {
        Ok(scalar) => scalar,
        Err(_) => unreachable!(),
    };

    let bridge_proof_deposit_key = match bridge_proof_master_key.add_tweak(secp256k1::SECP256K1, &deposit_index_scalar) {
        Ok((key, _parity)) => key,
        Err(_) => return Err("deposit index is negation of discrete logarithm of bridge proof master key (this is impossible for secure a master key)"),
    };

    if bridge_proof_tx.input.is_empty() {
        return Err("bridge proof transaction must have at least one input");
    }

    let bridge_proof_signature =
        extract_schnorr_signature_sighash_default(&bridge_proof_tx.input[0].witness)?;

    let mut sighash_cache = sighash::SighashCache::new(&bridge_proof_tx);
    let bridge_proof_sighash = sighash_cache
        .taproot_key_spend_signature_hash(
            0,
            &sighash::Prevouts::All(bridge_proof_prevouts.as_ref()),
            bitcoin::TapSighashType::Default,
        )
        .map_err(|_| "taproot error")?;

    let bridge_proof_sighash_msg = secp256k1::Message::from(bridge_proof_sighash);

    if secp256k1::SECP256K1
        .verify_schnorr(
            &bridge_proof_signature,
            &bridge_proof_sighash_msg,
            &bridge_proof_deposit_key,
        )
        .is_err()
    {
        return Err("bridge proof tx failed signature check");
    }

    if bridge_proof_tx.output.is_empty() {
        return Err("bridge proof transaction must have at least one output");
    }

    let bridge_proof_data = extract_op_return_data(&bridge_proof_tx.output[0])?;
    // FIXME: Extract pow from public values
    let mut public_values = [0; 36];
    public_values.copy_from_slice(&bridge_proof_data[0..36]);
    let mut acc_pow_high_bytes = [0; 4];
    acc_pow_high_bytes.copy_from_slice(&bridge_proof_data[32..32 + 4]);
    let acc_pow_high_bytes = u32::from_be_bytes(acc_pow_high_bytes);
    // TODO: Reduce to 128 bytes using compressed Groth proof
    //       This requires changes in zkaleido
    let mut bridge_proof_bytes = [0; 256];
    bridge_proof_bytes.copy_from_slice(&bridge_proof_data[32 + 4..]);

    match mode {
        CounterproofMode::InvalidBridgeProof => {
            let proof = Proof::new(bridge_proof_bytes.to_vec());
            // FIXME: Create public values from bridge proof tx
            // Same number of bytes as bridge proof public values
            let public_values = PublicValues::new(Vec::new());
            let proof_receipt = ProofReceipt::new(proof, public_values);
            // TODO: Move Buf32 into constant (Buf32::new needs to become const fn first)
            // TODO: Add SP1 key of dummy statement for testing
            //       We need a valid proof and an invalid proof for the given statement to put into
            // the bridge proof tx.
            let proof_vk = RollupVerifyingKey::NativeVerifyingKey;
            if groth16_verifier::verify_rollup_groth16_proof_receipt(&proof_receipt, &proof_vk)
                .is_ok()
            {
                // FIXME:
                // return Err("bridge proof should be invalid for counterproof to be valid");
            }
        }
        CounterproofMode::HeavierChain(heavier_chain) => {
            println!("User acc pow {}", acc_pow_high_bytes);
            let heavier_acc_pow = verify_header_chain(heavier_chain)?;
            let x = heavier_acc_pow.to_be_bytes();
            let heavier_acc_pow_high_bytes = u32::from_be_bytes([x[0], x[1], x[2], x[3]]);
            if heavier_acc_pow_high_bytes <= acc_pow_high_bytes {
                return Err(
                    "heavier chain must have more accumulated work than the operator chain",
                );
            }
        }
    }

    Ok(CounterproofPublicOutput {
        bridge_proof_master_key: bridge_proof_master_key.serialize(),
        deposit_index,
    })
}

/// Verifies that the given header chain is valid under Bitcoin consensus rules.
/// Returns the accumulated proof of work.
fn verify_header_chain(_chain: Vec<[u8; 80]>) -> Result<bitcoin::Work, &'static str> {
    // TODO: Construct header verification state without requiring rollup parameters.
    //       This function does NOT care about any L2 stuff!
    // TODO: Verify genesis header: not infinite pow
    //       Latest part of recursive proof
    // let btc_params = Params::new(rollup_params.network);
    // for header in &headers {
    //     header_vs.check_and_update_continuity(header, &btc_params)?;
    // }
    Ok(bitcoin::Work::from_be_bytes([0xff; 32]))
}

/// Extracts a Schnorr signature from a Taproot witness, ensuring it uses SIGHASH_DEFAULT.
fn extract_schnorr_signature_sighash_default(
    witness: &bitcoin::Witness,
) -> Result<schnorr::Signature, &'static str> {
    if witness.len() != 1 {
        return Err("witness must have length 1 (taproot key path spend)");
    }
    let sig_bytes = &witness[0];
    if sig_bytes.len() != 64 {
        return Err("sighash mode must be SIGHASH_DEFAULT, which means that the signature must be exactly 64 bytes");
    }

    schnorr::Signature::from_slice(sig_bytes).map_err(|_| "invalid signature")
}

fn extract_op_return_data(txout: &bitcoin::TxOut) -> Result<&[u8], &'static str> {
    let script = &txout.script_pubkey;
    if !script.is_op_return() {
        return Err("locking script must be an OP_RETURN script");
    }
    // OP_RETURN OP_PUSHDATA2 <32 + 4 + 256 = 292> (292 additional bytes...)
    //  32 bytes: tip hash
    //   4 bytes: accumulated proof of work (high bytes)
    // 256 bytes: Groth16 proof
    // Total length = 2 + 2 + 292 = 296
    if script.as_bytes().len() != 296 {
        return Err("OP_RETURN script must push exactly 292 bytes");
    }
    let data = &script.as_bytes()[2 + 2..];
    debug_assert_eq!(data.len(), 292);

    Ok(data)
}
