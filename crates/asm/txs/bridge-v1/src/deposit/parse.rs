use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;

use crate::{
    constants::BridgeTxType,
    deposit::{DEPOSIT_OUTPUT_INDEX, DepositInfo, aux::DepositTxHeaderAux},
    errors::TxStructureError,
};

/// Index of the deposit transaction input that spends the DRT (Deposit Request Transaction)
/// output.
const DRT_INPUT_INDEX: usize = 0;

/// Parses deposit transaction to extract [`DepositInfo`].
///
/// Parses a deposit transaction following the SPS-50 specification and extracts the decoded
/// auxiliary data ([`DepositTxHeaderAux`]) along with the deposit output and DRT inpoint. The
/// auxiliary data is encoded with [`strata_codec::Codec`] and includes the deposit index.
///
/// # Errors
///
/// Returns [`TxStructureError`] if the auxiliary data cannot be decoded or if the expected
/// deposit output at index [`DEPOSIT_OUTPUT_INDEX`] is missing.
pub fn parse_deposit_tx<'a>(tx_input: &TxInputRef<'a>) -> Result<DepositInfo, TxStructureError> {
    // Parse auxiliary data
    let header_aux: DepositTxHeaderAux = decode_buf_exact(tx_input.tag().aux_data())
        .map_err(|e| TxStructureError::invalid_auxiliary_data(BridgeTxType::Deposit, e))?;

    // Extract the deposit output
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_output(
                BridgeTxType::Deposit,
                DEPOSIT_OUTPUT_INDEX,
                "deposit output",
            )
        })?
        .clone()
        .into();

    // Extract the DRT inpoint
    let drt_inpoint = tx_input
        .tx()
        .input
        .get(DRT_INPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_input(BridgeTxType::Deposit, DRT_INPUT_INDEX, "drt input")
        })?
        .previous_output
        .into();

    // Construct the validated deposit information
    Ok(DepositInfo::new(header_aux, deposit_output, drt_inpoint))
}

#[cfg(test)]
mod tests {
    use std::mem;

    use bitcoin::{OutPoint, ScriptBuf, Transaction, TxOut, secp256k1::SECP256K1};
    use strata_crypto::test_utils::schnorr::create_agg_pubkey_from_privkeys;
    use strata_primitives::l1::BitcoinAmount;
    use strata_test_utils::ArbitraryGenerator;

    use crate::{
        constants::BridgeTxType,
        deposit::{
            DEPOSIT_OUTPUT_INDEX, DepositInfo, DepositTxHeaderAux, parse::DRT_INPUT_INDEX,
            parse_deposit_tx,
        },
        deposit_request::{DRT_OUTPUT_INDEX, DrtHeaderAux},
        errors::TxStructureErrorKind,
        test_utils::{
            create_connected_drt_and_dt, create_test_operators, mutate_aux_data, parse_sps50_tx,
        },
    };

    const AUX_LEN: usize = mem::size_of::<DepositTxHeaderAux>();

    fn create_deposit_tx_with_info() -> (DepositInfo, Transaction) {
        let mut arb = ArbitraryGenerator::new();
        let drt_header_aux: DrtHeaderAux = arb.generate();
        let deposit_idx: u32 = arb.generate();
        let amount = BitcoinAmount::from_sat(100_000);

        let (sks, _) = create_test_operators(3);
        let dt_aux = DepositTxHeaderAux::new(deposit_idx, *drt_header_aux.ee_address());
        let (drt, dt) =
            create_connected_drt_and_dt(&drt_header_aux, dt_aux.clone(), amount.into(), &sks);
        let nn_key = create_agg_pubkey_from_privkeys(&sks);

        let drt_inpoint = OutPoint::new(drt.compute_txid(), DRT_OUTPUT_INDEX as u32);
        let deposit_output = TxOut {
            value: amount.into(),
            script_pubkey: ScriptBuf::new_p2tr(SECP256K1, nn_key, None),
        };
        let info = DepositInfo::new(dt_aux, deposit_output.into(), drt_inpoint.into());
        (info, dt)
    }

    #[test]
    fn test_parse_dt_success() {
        let (info, tx) = create_deposit_tx_with_info();
        let tx_input = parse_sps50_tx(&tx);

        let parsed = parse_deposit_tx(&tx_input).expect("should parse deposit tx");

        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_missing_output() {
        let (_, mut tx) = create_deposit_tx_with_info();

        // Remove the deposit output
        tx.output.pop();

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Deposit);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingOutput {
                index: DEPOSIT_OUTPUT_INDEX
            }
        ))
    }

    #[test]
    fn test_parse_missing_input() {
        let (_, mut tx) = create_deposit_tx_with_info();

        // Remove all the inputs
        tx.input.clear();

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Deposit);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingInput {
                index: DRT_INPUT_INDEX
            }
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let (_, mut tx) = create_deposit_tx_with_info();

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Deposit);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_deposit_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::Deposit);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));
    }
}
