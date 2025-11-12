use arbitrary::{Arbitrary, Unstructured};
use bitcoin::ScriptBuf;
use strata_asm_common::TxInputRef;
use strata_primitives::l1::BitcoinOutPoint;

use super::{BRIDGE_INPUT_INDEX, USER_WITHDRAWAL_OUTPUT_INDEX};
use crate::{constants::COOPERATIVE_TX_TYPE, errors::CooperativeParseError};

const DEPOSIT_IDX_SIZE: usize = 4;

const DEPOSIT_IDX_OFFSET: usize = 0;

/// Minimum length of auxiliary data for cooperative transactions.
pub const COOPERATIVE_TX_AUX_DATA_LEN: usize = DEPOSIT_IDX_SIZE;

/// Information extracted from a Bitcoin cooperative transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooperativeInfo {
    /// The index of the deposit that the operator wishes to receive payout from later.
    /// This must be validated against the operator's assigned deposits in the state's assignments
    /// table to ensure the operator is authorized to claim this specific deposit.
    pub deposit_idx: u32,

    /// The UTXO of the deposit being spent in this cooperative transaction.
    pub deposit_utxo: BitcoinOutPoint,

    /// The scriptPubKey where the withdrawal funds are being sent.
    pub withdrawal_destination: ScriptBuf,
}

impl<'a> Arbitrary<'a> for CooperativeInfo {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        use strata_primitives::bitcoin_bosd::Descriptor;

        let withdrawal_destination = Descriptor::arbitrary(u)?.to_script();
        Ok(CooperativeInfo {
            deposit_idx: u32::arbitrary(u)?,
            deposit_utxo: BitcoinOutPoint::arbitrary(u)?,
            withdrawal_destination,
        })
    }
}

/// Parses cooperative withdrawal transaction to extract [`CooperativeInfo`].
///
/// Parses a cooperative withdrawal transaction following the SPS-50 specification and extracts
/// the withdrawal information . See the module-level documentation for the complete transaction
/// structure.
///
/// The function validates the transaction structure and parses the auxiliary data containing:
/// - Deposit index (4 bytes, big-endian u32)
///
/// # Parameters
///
/// - `tx` - Reference to the transaction input containing the withdrawal transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(CooperativeInfo)` - Successfully parsed cooperative withdrawal information
/// - `Err(CooperativeParseError)` - If the transaction structure is invalid, has insufficient
///   outputs, invalid metadata size, or any parsing step encounters malformed data
///
/// # Errors
///
/// This function will return an error if:
/// - The transaction has fewer than 2 outputs (missing withdrawal or OP_RETURN)
/// - The auxiliary data size doesn't match the expected metadata size
/// - Any of the metadata fields cannot be parsed correctly
pub fn parse_cooperative_tx<'t>(
    tx: &TxInputRef<'t>,
) -> Result<CooperativeInfo, CooperativeParseError> {
    if tx.tag().tx_type() != COOPERATIVE_TX_TYPE {
        return Err(CooperativeParseError::InvalidTxType(tx.tag().tx_type()));
    }

    let aux_data = tx.tag().aux_data();

    if aux_data.len() != COOPERATIVE_TX_AUX_DATA_LEN {
        return Err(CooperativeParseError::InvalidAuxiliaryData(aux_data.len()));
    }

    let bridge_input = &tx
        .tx()
        .input
        .get(BRIDGE_INPUT_INDEX)
        .ok_or(CooperativeParseError::MissingBridgeInput)?;

    let withdrawal_output = &tx
        .tx()
        .output
        .get(USER_WITHDRAWAL_OUTPUT_INDEX)
        .ok_or(CooperativeParseError::MissingWithdrawalOutput)?;

    let mut deposit_idx_bytes = [0u8; DEPOSIT_IDX_SIZE];
    deposit_idx_bytes
        .copy_from_slice(&aux_data[DEPOSIT_IDX_OFFSET..DEPOSIT_IDX_OFFSET + DEPOSIT_IDX_SIZE]);
    let deposit_idx = u32::from_be_bytes(deposit_idx_bytes);

    let deposit_utxo = bridge_input.previous_output;

    let withdrawal_destination = withdrawal_output.script_pubkey.clone();

    Ok(CooperativeInfo {
        deposit_idx,
        deposit_utxo: deposit_utxo.into(),
        withdrawal_destination,
    })
}

#[cfg(test)]
mod tests {
    use strata_asm_common::TxInputRef;
    use strata_l1_txfmt::ParseConfig;
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::{
        BRIDGE_V1_SUBPROTOCOL_ID,
        test_utils::{
            TEST_MAGIC_BYTES, create_tagged_payload, create_test_cooperative_tx,
            mutate_op_return_output, parse_tx,
        },
    };

    /// Tests that our hardcoded size constants match the actual type sizes.
    /// This is necessary to catch if the underlying types change size in the future,
    /// which would break the wire format compatibility for auxiliary data parsing.
    #[test]
    fn test_valid_size() {
        let deposit_idx_size: usize = std::mem::size_of::<u32>();
        assert_eq!(deposit_idx_size, DEPOSIT_IDX_SIZE);
    }

    #[test]
    fn test_parse_cooperative_fulfillment_tx_success() {
        let mut arb = ArbitraryGenerator::new();
        let info: CooperativeInfo = arb.generate();

        // Create the cooperative fulfillment transaction with proper SPS-50 format
        let tx = create_test_cooperative_tx(&info);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract cooperative info using the actual parser
        let extracted_info = parse_cooperative_tx(&tx_input_ref)
            .expect("Should successfully extract cooperative info");

        assert_eq!(extracted_info, info);
    }

    #[test]
    fn test_parse_cooperative_fulfillment_tx_invalid_type() {
        let mut arb = ArbitraryGenerator::new();
        let info: CooperativeInfo = arb.generate();

        let mut tx = create_test_cooperative_tx(&info);

        // Mutate the OP_RETURN output to have wrong transaction type
        let aux_data = vec![0u8; 40]; // Some dummy aux data
        let tagged_payload = create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, 99, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_cooperative_tx(&tx_input).unwrap_err();
        assert!(matches!(err, CooperativeParseError::InvalidTxType { .. }));
        if let CooperativeParseError::InvalidTxType(tx_type) = err {
            assert_eq!(tx_type, tx_input.tag().tx_type());
        }
    }

    #[test]
    fn test_parse_cooperative_fulfillment_tx_cooperative_output_missing() {
        let mut arb = ArbitraryGenerator::new();
        let info: CooperativeInfo = arb.generate();

        // Create the cooperative fulfillment transaction with proper SPS-50 format
        let mut tx = create_test_cooperative_tx(&info);
        // Remove the deposit output (keep only OP_RETURN at index 0)
        tx.output.truncate(1);

        // Parse the transaction using the SPS-50 parser
        let parser = ParseConfig::new(*TEST_MAGIC_BYTES);
        let tag_data = parser.try_parse_tx(&tx).expect("Should parse transaction");
        let tx_input_ref = TxInputRef::new(&tx, tag_data);

        // Extract cooperative info using the actual parser
        let err = parse_cooperative_tx(&tx_input_ref).unwrap_err();
        assert!(matches!(
            err,
            CooperativeParseError::MissingWithdrawalOutput
        ))
    }

    #[test]
    fn test_parse_cooperative_fulfillment_tx_invalid_aux_data() {
        let mut arb = ArbitraryGenerator::new();
        let info: CooperativeInfo = arb.generate();

        let mut tx = create_test_cooperative_tx(&info);

        // Mutate the OP_RETURN output to have shorter aux len
        let aux_data = vec![0u8; COOPERATIVE_TX_AUX_DATA_LEN - 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, COOPERATIVE_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_cooperative_tx(&tx_input).unwrap_err();

        assert!(matches!(
            err,
            CooperativeParseError::InvalidAuxiliaryData { .. }
        ));
        if let CooperativeParseError::InvalidAuxiliaryData(len) = err {
            assert_eq!(len, COOPERATIVE_TX_AUX_DATA_LEN - 1);
        }

        // Mutate the OP_RETURN output to have longer aux len
        let aux_data = vec![0u8; COOPERATIVE_TX_AUX_DATA_LEN + 1];
        let tagged_payload =
            create_tagged_payload(BRIDGE_V1_SUBPROTOCOL_ID, COOPERATIVE_TX_TYPE, aux_data);
        mutate_op_return_output(&mut tx, tagged_payload);

        let tx_input = parse_tx(&tx);
        let err = parse_cooperative_tx(&tx_input).unwrap_err();
        assert!(matches!(
            err,
            CooperativeParseError::InvalidAuxiliaryData { .. }
        ));
        if let CooperativeParseError::InvalidAuxiliaryData(len) = err {
            assert_eq!(len, COOPERATIVE_TX_AUX_DATA_LEN + 1);
        }
    }
}
