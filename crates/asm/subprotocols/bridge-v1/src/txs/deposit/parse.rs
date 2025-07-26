use bitcoin::{OutPoint, taproot::TAPROOT_CONTROL_NODE_SIZE};
use strata_asm_common::TxInputRef;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAmount, OutputRef},
};

use crate::{constants::DEPOSIT_TX_TYPE, errors::DepositError};

/// Length of the deposit index field in the auxiliary data (4 bytes for u32)
const DEPOSIT_IDX_LEN: usize = size_of::<u32>();
/// Length of the tapscript root hash in the auxiliary data (32 bytes)
const TAPSCRIPT_ROOT_LEN: usize = TAPROOT_CONTROL_NODE_SIZE;
/// Minimum length of auxiliary data (fixed fields only, excluding variable destination address)
const MIN_AUX_DATA_LEN: usize = DEPOSIT_IDX_LEN + TAPSCRIPT_ROOT_LEN;

const DEPOSIT_OUTPUT_INDEX: u32 = 1;

/// Information extracted from a Bitcoin deposit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositInfo {
    /// The index of the deposit in the bridge's deposit table.
    pub deposit_idx: u32,

    /// The amount of Bitcoin deposited.
    pub amt: BitcoinAmount,

    /// The destination address for the deposit.
    pub address: Vec<u8>,

    /// The outpoint of the deposit transaction.
    pub outpoint: OutputRef,

    /// The tapnode hash (merkle root) from the Deposit Request Transaction (DRT) being spent.
    ///
    /// This value is extracted from the auxiliary data and represents the merkle root of the
    /// tapscript tree from the DRT that this deposit transaction is spending. It is combined
    /// with the internal key (aggregated operator key) to reconstruct the taproot address
    /// that was used in the DRT's P2TR output.
    ///
    /// This is required to verify that the transaction was indeed signed by the claimed pubkey.
    /// Without this validation, someone could send funds to the N-of-N address without proper
    /// authorization, which would mint tokens but break the peg since there would be no presigned
    /// withdrawal transactions. This would require N-of-N trust for withdrawals instead of the
    /// intended 1-of-N trust assumption with presigned transactions.
    pub drt_tapnode_hash: Buf32,
}

/// Extracts deposit information from a Bitcoin bridge deposit transaction.
///
/// Parses a deposit transaction following the SPS-50 specification and extracts
/// the deposit information including amount, destination address, and validation data.
/// See the module-level documentation for the complete transaction structure.
///
/// # Parameters
///
/// - `tx_input` - Reference to the transaction input containing the deposit transaction and its
///   associated tag data
///
/// # Returns
///
/// - `Ok(DepositInfo)` - Successfully parsed deposit information
/// - `Err(DepositError)` - If the transaction structure is invalid, signature verification fails,
///   or any parsing step encounters malformed data
pub fn extract_deposit_info<'a>(tx_input: &TxInputRef<'a>) -> Result<DepositInfo, DepositError> {
    if tx_input.tag().tx_type() != DEPOSIT_TX_TYPE {
        return Err(DepositError::InvalidTxType {
            expected: DEPOSIT_TX_TYPE,
            actual: tx_input.tag().tx_type(),
        });
    }

    let aux_data = tx_input.tag().aux_data();

    // Validate minimum auxiliary data length (must have at least the fixed fields)
    if aux_data.len() < MIN_AUX_DATA_LEN {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Parse deposit index (bytes 0-3)
    let (deposit_idx_bytes, rest) = aux_data.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx = u32::from_be_bytes(
        deposit_idx_bytes
            .try_into()
            .expect("Expected deposit index to be 4 bytes"),
    );

    // Parse tapscript root hash (bytes 4-35)
    let (tapscript_root_bytes, destination_address) = rest.split_at(TAPSCRIPT_ROOT_LEN);
    let tapscript_root = Buf32::new(
        tapscript_root_bytes
            .try_into()
            .expect("Expected tapscript root to be 32 bytes"),
    );

    // Destination address is remaining bytes (bytes 36+)
    // Must have at least 1 byte for destination address
    if destination_address.is_empty() {
        return Err(DepositError::InvalidAuxiliaryData(aux_data.len()));
    }

    // Extract the deposit output (second output at index 1)
    let deposit_output = tx_input
        .tx()
        .output
        .get(DEPOSIT_OUTPUT_INDEX as usize)
        .ok_or(DepositError::MissingOutput(1))?;

    // Create outpoint reference for the deposit output
    let deposit_outpoint = OutputRef::from(OutPoint {
        txid: tx_input.tx().compute_txid(),
        vout: DEPOSIT_OUTPUT_INDEX,
    });

    // Construct the validated deposit information
    Ok(DepositInfo {
        deposit_idx,
        amt: deposit_output.value.into(),
        address: destination_address.to_vec(),
        outpoint: deposit_outpoint,
        drt_tapnode_hash: tapscript_root,
    })
}
