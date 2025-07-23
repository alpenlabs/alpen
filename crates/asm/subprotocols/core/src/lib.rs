//! # CoreASM Subprotocol
//!
//! This module implements the "CoreASM" subprotocol, responsible for
//! on-chain verification and anchoring of zk-SNARK checkpoint proofs.
//!
//! ## Overview
//!
//! The Core subprotocol is the central component of the Anchor State Machine (ASM)
//! that manages checkpoint verification and state transitions. It ensures that:
//!
//! - Each zk-SNARK proof of a new checkpoint is correctly verified
//! - State transitions follow the protocol rules
//! - Withdrawal messages are properly forwarded to the Bridge subprotocol
//! - Administrative keys (sequencer, verifying key) can be safely updated
//!
//! ## Key Components
//!
//! - **Checkpoint Verification**: Validates zk-SNARK proofs and state transitions
//! - **Message Handling**: Processes inter-subprotocol communications
//! - **State Management**: Maintains the latest verified checkpoint state
//! - **Withdrawal Processing**: Extracts and forwards L2→L1 withdrawal messages
//!
//! ## Transaction Types
//!
//! The Core subprotocol processes three types of transactions:
//!
//! 1. **OL STF Checkpoint** (`OL_STF_CHECKPOINT_TX_TYPE`): Contains signed checkpoint proofs
//! 2. **Forced Inclusion** (`FORCED_INCLUSION_TX_TYPE`): TBD
//! 3. **EE Upgrade** (`EE_UPGRADE_TX_TYPE`): TBD
//!
//! ## Security Considerations
//!
//! - All public parameters are constructed from trusted state, not sequencer input
//! - Signature verification prevents unauthorized checkpoint submissions
//! - State validation ensures proper progression of epochs and block heights
//! - Rolling hash verification prevents L1→L2 message manipulation

mod checkpoint_zk_verifier;
mod error;
mod logic;
mod utils;

use borsh::{BorshDeserialize, BorshSerialize, from_slice};
pub use error::*;
use strata_asm_common::{
    AnchorState, AsmError, AuxInputCollector, CORE_SUBPROTOCOL_ID, EE_UPGRADE_TX_TYPE,
    FORCED_INCLUSION_TX_TYPE, MsgRelayer, NullMsg, OL_STF_CHECKPOINT_TX_TYPE, Subprotocol,
    SubprotocolId, TxInputRef,
};
use strata_primitives::{
    batch::EpochSummary, buf::Buf32, l1::L1BlockId, proof::RollupVerifyingKey,
};

/// OL Core state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CoreOLState {
    /// The rollup verifying key used to verify each new checkpoint proof
    /// that has been posted on Bitcoin. Stored as serialized bytes for Borsh compatibility.
    checkpoint_vk_bytes: Vec<u8>,

    /// Summary of the last checkpoint that was successfully verified.
    /// New proofs are checked against this epoch summary.
    verified_checkpoint: EpochSummary,

    /// The L1 block ID up to which the `verified_checkpoint` covers.
    last_checkpoint_ref: L1BlockId,

    /// Public key of the sequencer authorized to submit checkpoint proofs.
    sequencer_pubkey: Buf32,
}

impl CoreOLState {
    /// Get the rollup verifying key by deserializing from stored bytes
    pub fn checkpoint_vk(&self) -> std::result::Result<RollupVerifyingKey, CoreError> {
        serde_json::from_slice(&self.checkpoint_vk_bytes)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))
    }

    /// Set the rollup verifying key by serializing to bytes
    pub fn set_checkpoint_vk(
        &mut self,
        vk: &RollupVerifyingKey,
    ) -> std::result::Result<(), CoreError> {
        self.checkpoint_vk_bytes = serde_json::to_vec(vk)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))?;
        Ok(())
    }
}

/// Genesis configuration for the Core subprotocol.
///
/// This structure contains all necessary parameters to properly initialize
/// the Core subprotocol state.
///
/// This struct sharing the same fields as CoreOLState but i create this
/// separately to avoid confusion (for now).
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CoreGenesisConfig {
    /// The initial checkpoint verifying key for zk-SNARK proof verification
    /// Stored as serialized bytes for Borsh compatibility.
    pub checkpoint_vk_bytes: Vec<u8>,

    /// The initial verified checkpoint state (usually genesis checkpoint)
    pub initial_checkpoint: EpochSummary,

    /// The initial L1 block reference for the checkpoint
    pub initial_l1_ref: L1BlockId,

    /// The authorized sequencer's public key for checkpoint submission
    pub sequencer_pubkey: Buf32,
}

impl CoreGenesisConfig {
    /// Create a new genesis config with the given rollup verifying key
    pub fn new(
        checkpoint_vk: &RollupVerifyingKey,
        initial_checkpoint: EpochSummary,
        initial_l1_ref: L1BlockId,
        sequencer_pubkey: Buf32,
    ) -> std::result::Result<Self, CoreError> {
        let checkpoint_vk_bytes = serde_json::to_vec(checkpoint_vk)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))?;

        Ok(Self {
            checkpoint_vk_bytes,
            initial_checkpoint,
            initial_l1_ref,
            sequencer_pubkey,
        })
    }

    /// Get the rollup verifying key by deserializing from stored bytes
    pub fn checkpoint_vk(&self) -> std::result::Result<RollupVerifyingKey, CoreError> {
        serde_json::from_slice(&self.checkpoint_vk_bytes)
            .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))
    }
}

/// OL Core subprotocol.
///
/// The OL Core subprotocol ensures that each zk‐SNARK proof of a new checkpoint
/// is correctly verified against the last known checkpoint state anchored on L1.
/// It manages the verifying key, tracks the latest verified checkpoint, and
/// enforces administrative controls over batch producer and consensus manager keys.
#[derive(Copy, Clone, Debug)]
pub struct OLCoreSubproto;

impl Subprotocol for OLCoreSubproto {
    const ID: SubprotocolId = CORE_SUBPROTOCOL_ID;

    type State = CoreOLState;

    type Msg = NullMsg<CORE_SUBPROTOCOL_ID>;

    type AuxInput = ();

    type GenesisConfig = CoreGenesisConfig;

    fn init(genesis_config_data: Option<&[u8]>) -> std::result::Result<Self::State, AsmError> {
        // Core subprotocol requires genesis configuration for proper initialization
        let genesis_data =
            genesis_config_data.ok_or(AsmError::MissingGenesisConfig(Self::ID))?;

        // Deserialize the genesis configuration
        let genesis_config: Self::GenesisConfig =
            from_slice(genesis_data).map_err(|e| AsmError::Deserialization(Self::ID, e))?;

        // Initialize the Core subprotocol state from genesis configuration
        Ok(CoreOLState {
            checkpoint_vk_bytes: genesis_config.checkpoint_vk_bytes,
            verified_checkpoint: genesis_config.initial_checkpoint,
            last_checkpoint_ref: genesis_config.initial_l1_ref,
            sequencer_pubkey: genesis_config.sequencer_pubkey,
        })
    }

    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _collector: &mut impl AuxInputCollector,
        _anchor_pre: &AnchorState,
    ) {
        // No auxiliary input needed for core subprotocol processing
    }

    // Transactions come from L1 and can be submitted by anyone, so we handle failures gracefully.
    // Invalid transactions are logged and ignored rather than causing panics or halting processing.
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        _aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    ) {
        for tx in txs {
            let result = match tx.tag().tx_type() {
                OL_STF_CHECKPOINT_TX_TYPE => handle_ol_stf_checkpoint(state, tx, relayer),
                FORCED_INCLUSION_TX_TYPE => handle_forced_inclusion(state, tx, relayer),
                EE_UPGRADE_TX_TYPE => handle_ee_upgrade(state, tx, relayer),

                // Ignore unknown transaction types
                _ => Err(CoreError::TxParsingError("unsupported tx type".to_string())),
            };

            // TODO: Implement proper logging approach
            // Since this code also runs as a part of zkVM guest program, we cannot use the
            // `tracing` crate. We need a proper logging mechanism to identify which
            // transaction processing failed and why. For now, we print errors to stderr
            // as a temporary solution.
            //
            // We can't propagate errors to upper layers when transaction processing fails because
            // invalidating and rejecting transactions is normal and expected behavior. We don't
            // want to halt the entire block processing because of a single invalid transaction.
            if let Err(e) = result {
                let txid = tx.tx().compute_txid();
                eprintln!("Error processing transaction (txid: {txid:?}): {e:?}");
            }
        }
    }

    fn process_msgs(_state: &mut Self::State, _msgs: &[Self::Msg]) {
        // TODO: Implement message processing from upgrade subprotocol messages
        // to update verifying key and sequencer key.
    }
}

fn handle_ol_stf_checkpoint(
    state: &mut CoreOLState,
    tx: &TxInputRef<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<()> {
    logic::ol_stf_checkpoint_handler(state, tx, relayer)
}

fn handle_forced_inclusion(
    _state: &mut CoreOLState,
    _tx: &TxInputRef<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // TODO: Implement forced inclusion transaction handling
    Ok(())
}

fn handle_ee_upgrade(
    _state: &mut CoreOLState,
    _tx: &TxInputRef<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // TODO: Implement execution environment upgrade transaction handling
    Ok(())
}
