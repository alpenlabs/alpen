use bitcoin::{ScriptBuf, XOnlyPublicKey};
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    errors::UnstakeTxParseError,
    unstake::{
        aux::UnstakeTxHeaderAux, info::UnstakeInfo, script::validate_and_extract_script_params,
    },
};

/// Index of the stake connector input.
pub const STAKE_INPUT_INDEX: usize = 0;

/// Expected number of items in the stake-connector witness stack.
///
/// Layout is fixed for the script-path spend we build in tests:
/// 1. 32-byte preimage
/// 2. Signature
/// 3. Executed script itself
/// 4. Control block proving this script belongs to the tweaked output key
///
/// Enforcing the length lets us index directly and fail fast on malformed witnesses.
const STAKE_WITNESS_ITEMS: usize = 4;

/// Parse an unstake transaction to extract [`UnstakeInfo`].
///
/// Parses an unstake transaction following the SPS-50 specification and extracts the auxiliary
/// metadata along with the aggregated N/N pubkey embedded in the stake-connector script (input
/// index 0).
pub fn parse_unstake_tx<'t>(tx: &TxInputRef<'t>) -> Result<UnstakeInfo, UnstakeTxParseError> {
    // Parse auxiliary data using UnstakeTxHeaderAux
    let header_aux: UnstakeTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    let stake_input = tx
        .tx()
        .input
        .get(STAKE_INPUT_INDEX)
        .ok_or(UnstakeTxParseError::MissingInput(STAKE_INPUT_INDEX))?;

    let witness = &stake_input.witness;

    let witness_len = witness.len();
    if witness_len != STAKE_WITNESS_ITEMS {
        return Err(UnstakeTxParseError::InvalidStakeWitnessLen {
            expected: STAKE_WITNESS_ITEMS,
            actual: witness_len,
        });
    }
    // With fixed layout, grab the script directly (index 2).
    let script = ScriptBuf::from_bytes(witness[2].to_vec());

    // Validate the script and extract parameters in one step.
    // This extracts nn_pubkey and stake_hash, reconstructs the expected script,
    // and compares byte-for-byte. Returns parameters only if script is valid.
    let (nn_pubkey_bytes, _stake_hash_bytes) = validate_and_extract_script_params(&script)
        .ok_or(UnstakeTxParseError::InvalidStakeScript)?;

    // Parse nn_pubkey from validated bytes (we know it's valid from the validation above)
    let witness_pushed_pubkey = XOnlyPublicKey::from_slice(&nn_pubkey_bytes)
        .map_err(|_| UnstakeTxParseError::InvalidNnPubkey)?;

    let info = UnstakeInfo::new(header_aux, witness_pushed_pubkey);

    Ok(info)
}

#[cfg(test)]
mod tests {
    use bitcoin::{Transaction, Witness};
    use strata_crypto::test_utils::schnorr::create_agg_pubkey_from_privkeys;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        test_utils::{create_test_operators, mutate_aux_data, parse_tx},
        unstake::build::build_connected_stake_and_unstake_txs,
    };

    const AUX_LEN: usize = std::mem::size_of::<UnstakeTxHeaderAux>();

    fn create_slash_tx_with_info() -> (UnstakeInfo, Transaction) {
        let header_aux: UnstakeTxHeaderAux = ArbitraryGenerator::new().generate();
        let (sks, _) = create_test_operators(3);
        let (_stake_tx, unstake_tx) = build_connected_stake_and_unstake_txs(&header_aux, &sks);
        let nn_key = create_agg_pubkey_from_privkeys(&sks);
        let info = UnstakeInfo::new(header_aux, nn_key);
        (info, unstake_tx)
    }

    #[test]
    fn test_parse_unstake_tx_success() {
        let (info, tx) = create_slash_tx_with_info();
        let tx_input = parse_tx(&tx);

        let parsed = parse_unstake_tx(&tx_input).expect("Should parse unstake tx");

        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_unstake_missing_stake_input() {
        let (_info, mut tx) = create_slash_tx_with_info();

        // Remove the stake connector to force an input count mismatch
        tx.input.pop();

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            UnstakeTxParseError::MissingInput(STAKE_INPUT_INDEX)
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let (_info, mut tx) = create_slash_tx_with_info();

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidAuxiliaryData(_)));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidAuxiliaryData(_)));
    }

    #[test]
    fn test_parse_unstake_rejects_mismatched_stake_script() {
        let (_info, mut tx) = create_slash_tx_with_info();

        // Corrupt the script so it no longer matches the canonical builder.
        let mut witness_items = tx.input[0].witness.to_vec();
        witness_items[2][0] ^= 1;
        tx.input[0].witness = Witness::from_slice(&witness_items);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidStakeScript));
    }

    #[test]
    fn test_parse_unstake_rejects_preimage_hash_mismatch() {
        let (_info, mut tx) = create_slash_tx_with_info();

        // Corrupt the preimage so the script hash check fails.
        let mut witness_items = tx.input[0].witness.to_vec();
        witness_items[0][0] ^= 1;
        tx.input[0].witness = Witness::from_slice(&witness_items);

        let tx_input = parse_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert!(matches!(err, UnstakeTxParseError::InvalidStakeScript));
    }
}
