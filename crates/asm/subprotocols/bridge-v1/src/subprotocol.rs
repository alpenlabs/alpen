//! Bridge V1 Subprotocol Implementation
//!
//! This module contains the core subprotocol implementation that integrates
//! with the Strata Anchor State Machine (ASM).

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{
    AnchorState, AsmError, AuxInputCollector, MsgRelayer, NullMsg, Subprotocol, SubprotocolId,
    TxInputRef,
};

use crate::{
    constants::{BRIDGE_V1_SUBPROTOCOL_ID, DEPOSIT_TX_TYPE, WITHDRAWAL_TX_TYPE},
    state::BridgeV1State,
    txs::{deposit::extract_deposit_info, withdrawal::extract_withdrawal_info},
};

/// Genesis configuration for the BridgeV1 subprotocol.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct BridgeV1GenesisConfig {
    /// Initial operator table for the bridge
    pub operators: crate::state::OperatorTable,
    /// Expected deposit amount for validation
    pub amount: strata_primitives::l1::BitcoinAmount,
}

/// Bridge V1 subprotocol implementation.
///
/// This struct implements the [`Subprotocol`] trait to integrate the bridge functionality
/// with the ASM. It handles Bitcoin deposit processing, operator management, and withdrawal
/// coordination.
#[derive(Copy, Clone, Debug)]
pub struct BridgeV1Subproto;

impl Subprotocol for BridgeV1Subproto {
    const ID: SubprotocolId = BRIDGE_V1_SUBPROTOCOL_ID;

    type State = BridgeV1State;

    type Msg = NullMsg<BRIDGE_V1_SUBPROTOCOL_ID>;

    type AuxInput = ();

    type GenesisConfig = BridgeV1GenesisConfig;

    fn init(genesis_config: Self::GenesisConfig) -> std::result::Result<Self::State, AsmError> {
        Ok(BridgeV1State::new(genesis_config.operators, genesis_config.amount))
    }

    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _collector: &mut impl AuxInputCollector,
        _anchor_pre: &AnchorState,
    ) {
        // No auxiliary input needed for bridge subprotocol processing
    }

    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        _relayer: &mut impl MsgRelayer,
    ) {
        for tx in txs {
            match tx.tag().tx_type() {
                DEPOSIT_TX_TYPE => Self::process_deposit_tx(state, tx),
                WITHDRAWAL_TX_TYPE => Self::process_withdrawal_tx(state, tx),
                _ => continue, // Ignore unsupported transaction types
            }
        }
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
        // TODO: Implement bridge message processing
    }
}

impl BridgeV1Subproto {
    /// Processes a deposit transaction with error logging.
    fn process_deposit_tx(state: &mut BridgeV1State, tx: &strata_asm_common::TxInputRef<'_>) {
        let deposit_info = match extract_deposit_info(tx) {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to extract deposit information from transaction"
                );
                return;
            }
        };

        let deposit_idx = match state.process_deposit(tx.tx(), &deposit_info) {
            Ok(idx) => idx,
            Err(e) => {
                tracing::error!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to process deposit"
                );
                return;
            }
        };

        tracing::info!(
            tx_id = %tx.tx().compute_txid(),
            deposit_idx = deposit_idx,
            amount = %deposit_info.amt,
            "Successfully processed deposit"
        );
    }

    /// Processes a withdrawal transaction with error logging.
    fn process_withdrawal_tx(state: &mut BridgeV1State, tx: &strata_asm_common::TxInputRef<'_>) {
        let withdrawal_info = match extract_withdrawal_info(tx) {
            Ok(info) => info,
            Err(e) => {
                tracing::warn!(
                    tx_id = %tx.tx().compute_txid(),
                    error = %e,
                    "Failed to extract withdrawal information from transaction"
                );
                return;
            }
        };

        if let Err(e) = state.process_withdrawal(&withdrawal_info) {
            tracing::warn!(
                tx_id = %tx.tx().compute_txid(),
                deposit_idx = withdrawal_info.deposit_idx(),
                operator_idx = withdrawal_info.operator_idx(),
                error = %e,
                "Withdrawal validation failed"
            );
            return;
        }

        tracing::info!(
            tx_id = %tx.tx().compute_txid(),
            deposit_idx = withdrawal_info.deposit_idx(),
            operator_idx = withdrawal_info.operator_idx(),
            amount = %withdrawal_info.withdrawal_amount(),
            "Successfully validated withdrawal"
        );
    }
}
