use bitcoin::Transaction;
use strata_codec::decode_buf_exact;
use strata_l1_txfmt::extract_tx_magic_and_tag;

use crate::{
    constants::BridgeTxType,
    deposit_request::{DRT_OUTPUT_INDEX, DepositRequestInfo, DrtHeaderAux},
    errors::TxStructureError,
};

/// Parses deposit request transaction to extract [`DepositRequestInfo`].
///
/// Parses a deposit request transaction following the SPS-50 specification and extracts the
/// decoded auxiliary data ([`DrtHeaderAux`]) along with the deposit amount. The
/// auxiliary data is encoded with [`strata_codec::Codec`] and includes the recovery public key
/// and destination address.
///
/// # Errors
///
/// Returns [`TxStructureError`] if the SPS-50 format cannot be parsed, the auxiliary
/// data cannot be decoded, or the expected deposit request output at index 1 is missing.
pub fn parse_drt(tx: &Transaction) -> Result<DepositRequestInfo, TxStructureError> {
    let (_magic, tag) = extract_tx_magic_and_tag(tx)
        .map_err(|e| TxStructureError::invalid_tx_format(BridgeTxType::DepositRequest, e))?;

    // Parse auxiliary data using DrtHeaderAux
    let aux_data: DrtHeaderAux = decode_buf_exact(tag.aux_data())
        .map_err(|e| TxStructureError::invalid_auxiliary_data(BridgeTxType::DepositRequest, e))?;

    // Extract the deposit request output (second output at index 1)
    let drt_output = tx
        .output
        .get(DRT_OUTPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_output(
                BridgeTxType::DepositRequest,
                DRT_OUTPUT_INDEX,
                "deposit request output",
            )
        })?
        .clone()
        .into();

    // Construct the validated deposit request information
    Ok(DepositRequestInfo::new(aux_data, drt_output))
}

#[cfg(test)]
mod tests {
    use std::mem;

    use bitcoin::Transaction;
    use strata_primitives::l1::BitcoinAmount;
    use strata_test_utils::ArbitraryGenerator;

    use crate::{
        constants::BridgeTxType,
        deposit::DepositTxHeaderAux,
        deposit_request::{DRT_OUTPUT_INDEX, DepositRequestInfo, DrtHeaderAux, parse_drt},
        errors::TxStructureErrorKind,
        test_utils::{create_connected_drt_and_dt, create_test_operators, mutate_aux_data},
    };

    const AUX_LEN: usize = mem::size_of::<DrtHeaderAux>();

    fn create_drt_tx_with_info() -> (DepositRequestInfo, Transaction) {
        let mut arb = ArbitraryGenerator::new();
        let drt_aux: DrtHeaderAux = arb.generate();
        let dt_aux: DepositTxHeaderAux = arb.generate();
        let amount = BitcoinAmount::from_sat(100_000);
        let (sks, _) = create_test_operators(3);

        let (drt, _dt) = create_connected_drt_and_dt(&drt_aux, dt_aux, amount.into(), &sks);
        let info = DepositRequestInfo::new(drt_aux, drt.output[DRT_OUTPUT_INDEX].clone().into());

        (info, drt)
    }

    #[test]
    fn test_parse_dt_success() {
        let (info, tx) = create_drt_tx_with_info();
        let parsed = parse_drt(&tx).expect("should parse deposit request tx");
        assert_eq!(info, parsed);
    }

    #[test]
    fn test_parse_missing_output() {
        let (_, mut tx) = create_drt_tx_with_info();

        // Remove the deposit output
        tx.output.pop();

        let err = parse_drt(&tx).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::DepositRequest);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingOutput {
                index: DRT_OUTPUT_INDEX
            }
        ))
    }

    #[test]
    fn test_parse_invalid_aux() {
        let (_, mut tx) = create_drt_tx_with_info();

        let larger_aux = [0u8; AUX_LEN + 1].to_vec();
        mutate_aux_data(&mut tx, larger_aux);

        let err = parse_drt(&tx).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::DepositRequest);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));

        let smaller_aux = [0u8; AUX_LEN - 1].to_vec();
        mutate_aux_data(&mut tx, smaller_aux);

        let err = parse_drt(&tx).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::DepositRequest);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));
    }
}
