use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_codec::decode_buf_exact;
use strata_primitives::l1::BitcoinAmount;

use crate::{
    errors::WithdrawalParseError,
    withdrawal_fulfillment::{
        USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX, aux::WithdrawalFulfillmentTxHeaderAux,
    },
};

/// Information extracted from a Bitcoin withdrawal fulfillment transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalFulfillmentInfo {
    /// Parsed SPS-50 auxiliary data.
    pub header_aux: WithdrawalFulfillmentTxHeaderAux,

    /// The Bitcoin script address where the withdrawn funds are being sent.
    pub withdrawal_destination: ScriptBuf,

    /// The amount of Bitcoin being withdrawn (may be less than the original deposit due to fees).
    pub withdrawal_amount: BitcoinAmount,
}

impl<'a> Arbitrary<'a> for WithdrawalFulfillmentInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        use strata_primitives::bitcoin_bosd::Descriptor;

        let withdrawal_destination = Descriptor::arbitrary(u)?.to_script();
        Ok(WithdrawalFulfillmentInfo {
            header_aux: WithdrawalFulfillmentTxHeaderAux::arbitrary(u)?,
            withdrawal_destination,
            withdrawal_amount: BitcoinAmount::from_sat(u64::arbitrary(u)?),
        })
    }
}

/// Parses withdrawal fulfillment transaction to extract [`WithdrawalFulfillmentInfo`].
///
/// Parses a withdrawal fulfillment transaction following the SPS-50 specification and extracts
/// the withdrawal fulfillment information including the deposit index, recipient address, and
/// withdrawal amount. See the module-level documentation for the complete transaction structure.
///
/// The function validates the transaction structure and parses the auxiliary data containing:
/// - Deposit index (4 bytes, big-endian u32) - identifies the locked deposit UTXO that the operator
///   will receive payout from after successful verification of assignment fulfillment
///
/// # Parameters
///
/// - `tx` - Reference to the transaction input containing the withdrawal fulfillment transaction
///   and its associated tag data
///
/// # Returns
///
/// - `Ok(WithdrawalFulfillmentInfo)` - Successfully parsed withdrawal fulfillment information
/// - `Err(WithdrawalParseError)` - If the transaction structure is invalid, has insufficient
///   outputs, invalid metadata size, or any parsing step encounters malformed data
///
/// # Errors
///
/// This function will return an error if:
/// - The transaction has fewer than 2 outputs (missing withdrawal fulfillment or OP_RETURN)
/// - The auxiliary data size doesn't match the expected metadata size
/// - Any of the metadata fields cannot be parsed correctly
pub fn parse_withdrawal_fulfillment_tx<'t>(
    tx: &TxInputRef<'t>,
) -> Result<WithdrawalFulfillmentInfo, WithdrawalParseError> {
    let header_aux: WithdrawalFulfillmentTxHeaderAux = decode_buf_exact(tx.tag().aux_data())?;

    let withdrawal_fulfillment_output = &tx
        .tx()
        .output
        .get(USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX)
        .ok_or(WithdrawalParseError::MissingUserFulfillmentOutput)?;

    let withdrawal_amount = BitcoinAmount::from_sat(withdrawal_fulfillment_output.value.to_sat());
    let withdrawal_destination = withdrawal_fulfillment_output.script_pubkey.clone();

    Ok(WithdrawalFulfillmentInfo {
        header_aux,
        withdrawal_destination,
        withdrawal_amount,
    })
}

#[cfg(test)]
mod tests {

    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::test_utils::{
        TEST_MAGIC_BYTES, create_test_withdrawal_fulfillment_tx, mutate_aux_data, parse_tx,
    };

    /// Minimum length of auxiliary data for withdrawal fulfillment transactions.
    const WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN: usize =
        std::mem::size_of::<WithdrawalFulfillmentTxHeaderAux>();

    #[test]
    fn test_parse_withdrawal_fulfillment_tx_success() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        // Create the withdrawal fulfillment transaction with proper SPS-50 format
        let tx = create_test_withdrawal_fulfillment_tx(&info);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
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
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract withdrawal info using the actual parser
        let err = parse_withdrawal_fulfillment_tx(&tx_input_ref).unwrap_err();
        assert!(matches!(
            err,
            WithdrawalParseError::MissingUserFulfillmentOutput
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

        let tx_input = parse_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();

        assert!(matches!(err, WithdrawalParseError::InvalidAuxiliaryData(_)));

        // Mutate the OP_RETURN output to have longer aux len - this should fail
        let long_aux_data = vec![0u8; WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN + 1];
        mutate_aux_data(&mut tx, long_aux_data);

        let tx_input = parse_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();
        assert!(matches!(err, WithdrawalParseError::InvalidAuxiliaryData(_)));
    }
}
