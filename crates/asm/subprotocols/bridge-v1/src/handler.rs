use bitcoin::{OutPoint, ScriptBuf, XOnlyPublicKey};
use secp256k1::SECP256K1;
use strata_asm_common::{AsmLogEntry, AuxRequestCollector, MsgRelayer, VerifiedAuxData};
use strata_asm_logs::NewExportEntry;
use strata_primitives::l1::BitcoinXOnlyPublicKey;

use crate::{
    errors::BridgeSubprotocolError,
    parser::{ParsedDepositTx, ParsedTx},
    state::BridgeV1State,
};

/// Handles parsed transactions and update the bridge state accordingly.
///
/// # Transaction Types and Log Behavior:
/// - **Deposit**: Processes the deposit transaction without emitting logs
/// - **WithdrawalFulfillment**: Processes the withdrawal and emits a withdrawal processed log via
///   the relayer to notify other components of the processed withdrawal
///
/// # Arguments
/// * `state` - Mutable reference to the bridge state to be updated
/// * `parsed_tx` - The parsed transaction to handle
/// * `relayer` - The message relayer used for emitting logs
///
/// # Returns
/// * `Ok(())` if the transaction was processed successfully
/// * `Err(BridgeSubprotocolError)` if an error occurred during processing
pub(crate) fn handle_parsed_tx<'t>(
    state: &mut BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    relayer: &mut impl MsgRelayer,
    aux_data: &VerifiedAuxData,
) -> Result<(), BridgeSubprotocolError> {
    match parsed_tx {
        ParsedTx::Deposit(parsed_deposit_tx) => {
            let ParsedDepositTx { tx, info } = parsed_deposit_tx;
            state.process_deposit_tx(tx, &info)?;
            Ok(())
        }
        ParsedTx::WithdrawalFulfillment(info) => {
            state.process_withdrawal_fulfillment_tx(&info)?;
            Ok(())
        }
        ParsedTx::Commit(parsed_commit_tx) => {
            validate_nn_spend(
                &parsed_commit_tx.prev_outpoint.0,
                state.operators().agg_key(),
                aux_data,
            )?;

            let unlock = state.process_commit_tx(&parsed_commit_tx)?;

            let container_id = 0; // Replace with actual logic to determine container ID
            let withdrawal_processed_log =
                NewExportEntry::new(container_id, unlock.to_export_entry());
            relayer.emit_log(AsmLogEntry::from_log(&withdrawal_processed_log).expect("FIXME:"));

            Ok(())
        }
    }
}

/// Validates that an outpoint is locked to an N-of-N multisig.
///
/// Verifies that the output referenced by `prev_outpoint` is locked to the expected
/// N-of-N aggregated operator key using P2TR key-spend only (no merkle root).
///
/// # Arguments
///
/// * `prev_outpoint` - The outpoint referencing the output to validate
/// * `nn_pubkey` - The expected N-of-N aggregated public key
/// * `aux_data` - Auxiliary data to retrieve the output
///
/// # Returns
///
/// * `Ok(())` if the output is locked to `nn_pubkey`
/// * `Err(BridgeSubprotocolError)` if validation fails
fn validate_nn_spend(
    prev_outpoint: &OutPoint,
    nn_pubkey: &BitcoinXOnlyPublicKey,
    aux_data: &VerifiedAuxData,
) -> Result<(), BridgeSubprotocolError> {
    // Retrieve the output being validated
    let prev_txout = aux_data.get_bitcoin_txout(prev_outpoint)?;

    // Build the expected P2TR script locked to the N-of-N key
    let secp = SECP256K1;
    let nn_xonly = XOnlyPublicKey::from_slice(nn_pubkey.inner().as_bytes())
        .map_err(|_| BridgeSubprotocolError::InvalidSpentOutputLock)?;
    let expected_script = ScriptBuf::new_p2tr(secp, nn_xonly, None);

    // Verify the output is locked to the expected N-of-N key
    if prev_txout.script_pubkey != expected_script {
        return Err(BridgeSubprotocolError::InvalidSpentOutputLock);
    }

    Ok(())
}

pub(crate) fn preprocess_parsed_tx<'t>(
    state: &BridgeV1State,
    parsed_tx: ParsedTx<'t>,
    collector: &mut AuxRequestCollector,
) {
    match parsed_tx {
        ParsedTx::Deposit(_) => {}
        ParsedTx::WithdrawalFulfillment(_) => {}
        ParsedTx::Commit(_) => {}
    }
}
