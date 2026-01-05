use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::BitcoinAmount;

use crate::{
    constants::BridgeTxType,
    errors::TxStructureError,
    withdrawal_fulfillment::{
        USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX, WithdrawalFulfillmentInfo,
        aux::WithdrawalFulfillmentTxHeaderAux,
    },
};

/// Parses withdrawal fulfillment transaction to extract [`WithdrawalFulfillmentInfo`].
///
/// Parses a withdrawal fulfillment transaction following the SPS-50 specification and extracts
/// the decoded auxiliary data ([`WithdrawalFulfillmentTxHeaderAux`]), recipient address, and
/// withdrawal amount. The auxiliary data is encoded with [`strata_codec::Codec`] and currently
/// contains only the deposit index that ties the payout to a specific assignment.
///
/// # Errors
///
/// Returns [`TxStructureError`] if the auxiliary data cannot be decoded into
/// [`WithdrawalFulfillmentTxHeaderAux`] or if the required withdrawal fulfillment output at index 1
/// is missing.
pub fn parse_withdrawal_fulfillment_tx<'t>(
    tx: &TxInputRef<'t>,
) -> Result<WithdrawalFulfillmentInfo, TxStructureError> {
    let header_aux: WithdrawalFulfillmentTxHeaderAux = decode_buf_exact(tx.tag().aux_data())
        .map_err(|e| {
            TxStructureError::invalid_auxiliary_data(BridgeTxType::WithdrawalFulfillment, e)
        })?;

    let withdrawal_fulfillment_output = &tx
        .tx()
        .output
        .get(USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX)
        .ok_or_else(|| {
            TxStructureError::missing_output(
                BridgeTxType::WithdrawalFulfillment,
                USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX,
                "user withdrawal fulfillment output",
            )
        })?;

    let withdrawal_amount = BitcoinAmount::from_sat(withdrawal_fulfillment_output.value.to_sat());
    let withdrawal_destination = withdrawal_fulfillment_output.script_pubkey.clone();

    Ok(WithdrawalFulfillmentInfo::new(
        header_aux,
        withdrawal_destination,
        withdrawal_amount,
    ))
}

#[cfg(test)]
mod tests {

    use std::mem;

    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        errors::TxStructureErrorKind,
        test_utils::{
            TEST_MAGIC_BYTES, create_test_withdrawal_fulfillment_tx, mutate_aux_data,
            parse_sps50_tx,
        },
    };

    /// Minimum length of auxiliary data for withdrawal fulfillment transactions.
    const WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN: usize =
        mem::size_of::<WithdrawalFulfillmentTxHeaderAux>();

    #[test]
    fn test_parse_withdrawal_fulfillment_tx_success() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        // Create the withdrawal fulfillment transaction with proper SPS-50 format
        let tx = create_test_withdrawal_fulfillment_tx(&info);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract withdrawal info using the actual parser
        let extracted_info = parse_withdrawal_fulfillment_tx(&tx_input_ref)
            .expect("Should successfully extract withdrawal info");

        assert_eq!(extracted_info, info);
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_tx_withdrawal_output_missing() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        // Create the withdrawal fulfillment transaction with proper SPS-50 format
        let mut tx = create_test_withdrawal_fulfillment_tx(&info);
        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract withdrawal info using the actual parser
        let err = parse_withdrawal_fulfillment_tx(&tx_input_ref).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::WithdrawalFulfillment);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::MissingOutput {
                index: USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX
            }
        ))
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_tx_invalid_aux_data() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        let mut tx = create_test_withdrawal_fulfillment_tx(&info);

        // Mutate the OP_RETURN output to have shorter aux len - this should fail
        let short_aux_data = vec![0u8; WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN - 1];
        mutate_aux_data(&mut tx, short_aux_data);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();

        assert_eq!(err.tx_type(), BridgeTxType::WithdrawalFulfillment);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));

        // Mutate the OP_RETURN output to have longer aux len - this should fail
        let long_aux_data = vec![0u8; WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN + 1];
        mutate_aux_data(&mut tx, long_aux_data);

        let tx_input = parse_sps50_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();
        assert_eq!(err.tx_type(), BridgeTxType::WithdrawalFulfillment);
        assert!(matches!(
            err.kind(),
            TxStructureErrorKind::InvalidAuxiliaryData(_)
        ));
    }
}
