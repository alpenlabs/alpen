use arbitrary::{Arbitrary, Unstructured};
use bitcoin::{ScriptBuf, Txid, hashes::Hash};
use strata_asm_common::TxInputRef;
use strata_bridge_types::OperatorIdx;
use strata_primitives::l1::{BitcoinAmount, BitcoinTxid};

use crate::{
    constants::WITHDRAWAL_TX_TYPE, errors::CooperativeParseError,
    withdrawal_fulfillment::USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX,
};

const OPERATOR_IDX_SIZE: usize = 4;
const DEPOSIT_IDX_SIZE: usize = 4;
const DEPOSIT_TXID_SIZE: usize = 32;

const OPERATOR_IDX_OFFSET: usize = 0;
const DEPOSIT_IDX_OFFSET: usize = OPERATOR_IDX_OFFSET + OPERATOR_IDX_SIZE;
const DEPOSIT_TXID_OFFSET: usize = DEPOSIT_IDX_OFFSET + DEPOSIT_IDX_SIZE;

/// Minimum length of auxiliary data for withdrawal fulfillment transactions.
pub const WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN: usize =
    OPERATOR_IDX_SIZE + DEPOSIT_IDX_SIZE + DEPOSIT_TXID_SIZE;

/// Information extracted from a Bitcoin withdrawal transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithdrawalFulfillmentInfo {
    /// The index of the operator who processed this withdrawal.
    pub operator_idx: OperatorIdx,

    /// The index of the deposit that the operator wishes to receive payout from later.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to claim this specific deposit.
    pub deposit_idx: u32,

    /// The transaction ID of the deposit that the operator wishes to claim for payout.
    /// This must match the deposit referenced by `deposit_idx` in the assignments table.
    pub deposit_txid: BitcoinTxid,

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
            operator_idx: u32::arbitrary(u)?,
            deposit_idx: u32::arbitrary(u)?,
            deposit_txid: BitcoinTxid::arbitrary(u)?,
            withdrawal_destination,
            withdrawal_amount: BitcoinAmount::from_sat(u64::arbitrary(u)?),
        })
    }
}

/// Parses withdrawal fulfillment transaction to extract [`WithdrawalFulfillmentInfo`].
///
/// Parses a withdrawal transaction following the SPS-50 specification and extracts
/// the withdrawal information including operator index, deposit references, recipient address,
/// and withdrawal amount. See the module-level documentation for the complete transaction
/// structure.
///
/// The function validates the transaction structure and parses the auxiliary data containing:
/// - Operator index (4 bytes, big-endian u32)
/// - Deposit index (4 bytes, big-endian u32)
/// - Deposit transaction ID (32 bytes)
///
/// # Parameters
///
/// - `tx` - Reference to the transaction input containing the withdrawal transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(WithdrawalInfo)` - Successfully parsed withdrawal information
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
) -> Result<WithdrawalFulfillmentInfo, CooperativeParseError> {
    if tx.tag().tx_type() != WITHDRAWAL_TX_TYPE {
        return Err(CooperativeParseError::InvalidTxType(tx.tag().tx_type()));
    }

    let withdrawal_auxdata = tx.tag().aux_data();

    if withdrawal_auxdata.len() != WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN {
        return Err(CooperativeParseError::InvalidAuxiliaryData(
            withdrawal_auxdata.len(),
        ));
    }

    let withdrawal_fulfillment_output = &tx
        .tx()
        .output
        .get(USER_WITHDRAWAL_FULFILLMENT_OUTPUT_INDEX)
        .ok_or(CooperativeParseError::MissingWithdrawalOutput)?;
    let mut operator_idx_bytes = [0u8; OPERATOR_IDX_SIZE];
    operator_idx_bytes.copy_from_slice(
        &withdrawal_auxdata[OPERATOR_IDX_OFFSET..OPERATOR_IDX_OFFSET + OPERATOR_IDX_SIZE],
    );
    let operator_idx = u32::from_be_bytes(operator_idx_bytes);

    let mut deposit_idx_bytes = [0u8; DEPOSIT_IDX_SIZE];
    deposit_idx_bytes.copy_from_slice(
        &withdrawal_auxdata[DEPOSIT_IDX_OFFSET..DEPOSIT_IDX_OFFSET + DEPOSIT_IDX_SIZE],
    );
    let deposit_idx = u32::from_be_bytes(deposit_idx_bytes);

    let mut deposit_txid_bytes = [0u8; DEPOSIT_TXID_SIZE];
    deposit_txid_bytes.copy_from_slice(
        &withdrawal_auxdata[DEPOSIT_TXID_OFFSET..DEPOSIT_TXID_OFFSET + DEPOSIT_TXID_SIZE],
    );
    let deposit_txid = Txid::from_byte_array(deposit_txid_bytes);

    let withdrawal_amount = BitcoinAmount::from_sat(withdrawal_fulfillment_output.value.to_sat());
    let withdrawal_destination = withdrawal_fulfillment_output.script_pubkey.clone();

    Ok(WithdrawalFulfillmentInfo {
        operator_idx,
        deposit_idx,
        deposit_txid: deposit_txid.into(),
        withdrawal_destination,
        withdrawal_amount,
    })
}

#[cfg(test)]
mod tests {

    use strata_asm_common::TxInputRef;
    use strata_bridge_types::OperatorIdx;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        BRIDGE_V1_SUBPROTOCOL_ID,
        test_utils::{
            TEST_MAGIC_BYTES, create_tagged_payload, create_test_withdrawal_fulfillment_tx,
            mutate_op_return_output, parse_tx,
        },
    };

    /// Tests that our hardcoded size constants match the actual type sizes.
    /// This is necessary to catch if the underlying types change size in the future,
    /// which would break the wire format compatibility for auxiliary data parsing.
    #[test]
    fn test_valid_size() {
        let operator_idx_size: usize = std::mem::size_of::<OperatorIdx>();
        assert_eq!(operator_idx_size, OPERATOR_IDX_SIZE);

        let deposit_idx_size: usize = std::mem::size_of::<u32>();
        assert_eq!(deposit_idx_size, DEPOSIT_IDX_SIZE);

        let deposit_txid_size: usize = std::mem::size_of::<Txid>();
        assert_eq!(deposit_txid_size, DEPOSIT_TXID_SIZE)
    }

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
    fn test_parse_withdrawal_fulfillment_tx_invalid_type() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        let mut tx = create_test_withdrawal_fulfillment_tx(&info);

        // Mutate the OP_RETURN output to have wrong transaction type
        let aux_data = vec![0u8; 40]; // Some dummy aux data
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();
        assert!(matches!(err, CooperativeParseError::InvalidTxType { .. }));
        if let CooperativeParseError::InvalidTxType(tx_type) = err {
            assert_eq!(tx_type, tx_input.tag().tx_type());
        }
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
            CooperativeParseError::MissingWithdrawalOutput
        ))
    }

    #[test]
    fn test_parse_withdrawal_fulfillment_tx_invalid_aux_data() {
        let mut arb = ArbitraryGenerator::new();
        let info: WithdrawalFulfillmentInfo = arb.generate();

        let mut tx = create_test_withdrawal_fulfillment_tx(&info);

        // Mutate the OP_RETURN output to have shorter aux len
        let aux_data = vec![0u8; WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN - 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();

        assert!(matches!(
            err,
            CooperativeParseError::InvalidAuxiliaryData { .. }
        ));
        if let CooperativeParseError::InvalidAuxiliaryData(len) = err {
            assert_eq!(len, WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN - 1);
        }

        // Mutate the OP_RETURN output to have longer aux len
        let aux_data = vec![0u8; WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN + 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, WITHDRAWAL_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_withdrawal_fulfillment_tx(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            CooperativeParseError::InvalidAuxiliaryData { .. }
        ));
        if let CooperativeParseError::InvalidAuxiliaryData(len) = err {
            assert_eq!(len, WITHDRAWAL_FULFILLMENT_TX_AUX_DATA_LEN + 1);
        }
    }
}
