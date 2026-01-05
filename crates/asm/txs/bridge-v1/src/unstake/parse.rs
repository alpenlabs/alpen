use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    constants::BridgeTxType,
    errors::{TxStructureError, WitnessError},
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

/// Index of the executed script in witness stack.
const SCRIPT_INDEX: usize = 2;

/// Parse an unstake transaction to extract [`UnstakeInfo`].
///
/// Parses an unstake transaction following the SPS-50 specification and extracts the auxiliary
/// metadata along with the aggregated N/N pubkey embedded in the stake-connector script (input
/// index 0).
pub fn parse_unstake_tx<'t>(tx: &TxInputRef<'t>) -> Result<UnstakeInfo, TxStructureError> {
    // Parse auxiliary data using UnstakeTxHeaderAux
    let header_aux: UnstakeTxHeaderAux = decode_buf_exact(tx.tag().aux_data())
        .map_err(|e| TxStructureError::invalid_auxiliary_data(BridgeTxType::Unstake, e))?;

    let stake_input = tx.tx().input.get(STAKE_INPUT_INDEX).ok_or_else(|| {
        TxStructureError::missing_input(
            BridgeTxType::Unstake,
            STAKE_INPUT_INDEX,
            "stake connector input",
        )
    })?;

    let witness = &stake_input.witness;

    let witness_len = witness.len();
    if witness_len != STAKE_WITNESS_ITEMS {
        return Err(TxStructureError::invalid_witness(
            BridgeTxType::Unstake,
            WitnessError::InvalidLength {
                expected: STAKE_WITNESS_ITEMS,
                actual: witness_len,
            },
            "stake connector witness",
        ));
    }
    // With fixed layout, grab the script directly.
    let script = ScriptBuf::from_bytes(witness[SCRIPT_INDEX].to_vec());

    // Validate the script and extract parameters in one step.
    // This extracts nn_pubkey and stake_hash, reconstructs the expected script,
    // and compares byte-for-byte. Returns parameters only if script is valid.
    let (witness_pushed_pubkey, _stake_hash_bytes) = validate_and_extract_script_params(&script)
        .ok_or_else(|| {
            TxStructureError::invalid_witness(
                BridgeTxType::Unstake,
                WitnessError::InvalidScriptStructure,
                "stake connector witness script",
            )
        })?;

    let info = UnstakeInfo::new(header_aux, witness_pushed_pubkey);

    Ok(info)
}

#[cfg(test)]
mod tests {
    use std::mem;

    use bitcoin::Transaction;
    use strata_crypto::test_utils::schnorr::create_agg_pubkey_from_privkeys;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        errors::TxStructureErrorKind,
        test_utils::{
            create_connected_stake_and_unstake_txs, create_test_operators, mutate_aux_data,
            parse_sps50_tx,
        },
    };

    const AUX_LEN: usize = mem::size_of::<UnstakeTxHeaderAux>();

    fn create_slash_tx_with_info() -> (UnstakeInfo, Transaction) {
        let header_aux: UnstakeTxHeaderAux = ArbitraryGenerator::new().generate();
        let (sks, _) = create_test_operators(3);
        let (_stake_tx, unstake_tx) = create_connected_stake_and_unstake_txs(&header_aux, &sks);
        let nn_key = create_agg_pubkey_from_privkeys(&sks);
        let info = UnstakeInfo::new(header_aux, nn_key);
        (info, unstake_tx)
    }

    #[test]
    fn test_parse_unstake_tx_success() {
        let (info, tx) = create_slash_tx_with_info();
        let tx_input = parse_sps50_tx(&tx);

        let parsed = parse_unstake_tx(&tx_input).expect("Should parse unstake tx");

        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_unstake_missing_stake_input() {
        let (_info, mut tx) = create_slash_tx_with_info();

        // Remove the stake connector
        tx.input.pop();

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Unstake);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingInput {
                index: STAKE_INPUT_INDEX
            }
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let (_info, mut tx) = create_slash_tx_with_info();

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Unstake);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_unstake_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Unstake);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));
    }
}
