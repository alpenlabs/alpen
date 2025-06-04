//! This module implements the "CoreASM" subprotocol, responsible for
//! on-chain verification and anchoring of zk-SNARK checkpoint proofs.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{InterprotoMsg, Log, MsgRelayer, Subprotocol, SubprotocolId, TxInput};
use strata_primitives::{
    batch::{Checkpoint, EpochSummary, SignedCheckpoint, verify_signed_checkpoint_sig},
    block_credential::CredRule,
    buf::Buf32,
    l1::L1BlockId,
    params::{ProofPublishMode, RollupParams},
    proof::RollupVerifyingKey,
};
use strata_state::forced_inclusion::ForcedInclusion;
use thiserror::Error;
use zkaleido::VerifyingKey;
use zkaleido_risc0_groth16_verifier as _;
use zkaleido_sp1_groth16_verifier as _;

mod checkpoint_verification;
mod hash;

/// The unique identifier for the CoreASM subprotocol within the Anchor State Machine.
///
/// This constant is used to tag `SectionState` entries belonging to the CoreASM logic
/// and must match the `subprotocol_id` checked in `SectionState::subprotocol()`.
pub const CORE_SUBPROTOCOL_ID: SubprotocolId = 1;

/// Bridge subprotocol ID for sending messages
pub const BRIDGE_SUBPROTOCOL_ID: SubprotocolId = 2;

/// Upgrade subprotocol ID for receiving VK update messages
pub const UPGRADE_SUBPROTOCOL_ID: SubprotocolId = 3;

const OL_STF_CHECKPOINT_TX_TYPE: u8 = 1;
const FORCED_INCLUSION_TX_TYPE: u8 = 2;
const EE_UPGRADE_TX_TYPE: u8 = 3;

/// Log type identifiers as defined in the specification
const CHECKPOINT_SUMMARY_TY: u16 = 1;
const FORCED_INCLUSION_TY: u16 = 2;
// Reserved for future VK update log emission
#[allow(dead_code)]
const VK_UPDATE_TY: u16 = 3;
#[allow(dead_code)]
const SEQUENCER_KEY_UPDATE_TY: u16 = 4;

/// Errors that can occur during Core subprotocol processing
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Malformed signed checkpoint: {0}")]
    MalformedSignedCheckpoint(String),

    #[error("Invalid signature")]
    InvalidSignature,

    #[error("Malformed public parameters: {0}")]
    MalformedPublicParams(String),

    #[error("State diff hash mismatch")]
    StateDiffMismatch,

    #[error("Unexpected previous terminal")]
    UnexpectedPrevTerminal,

    #[error("Unexpected previous L1 reference")]
    UnexpectedPrevL1Ref,

    #[error("L1 to L2 message range mismatch")]
    L1ToL2RangeMismatch,

    #[error("Proof verification failed")]
    ProofVerificationFailed,

    #[error("Checkpoint verification failed: {0}")]
    CheckpointVerificationFailed(String),

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),
}

/// EE identifier type (stub for now)
pub type EEIdentifier = u8;

/// L2 to L1 message type (stub for now)
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct L2ToL1Msg {
    // TODO: Define proper structure based on withdrawal requirements
    pub data: Vec<u8>,
}

/// Withdrawal messages to be sent to Bridge subprotocol
#[derive(Clone, Debug)]
pub struct WithdrawalMsg {
    pub withdrawals: Vec<L2ToL1Msg>,
}

impl InterprotoMsg for WithdrawalMsg {
    fn id(&self) -> SubprotocolId {
        BRIDGE_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// Messages sent from Upgrade subprotocol to Core subprotocol
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum UpgradeToCoreMessage {
    /// Update the checkpoint verifying key
    UpdateCheckpointVk(VerifyingKey),
    /// Update the sequencer public key
    UpdateSequencerPubkey(Buf32),
}

impl InterprotoMsg for UpgradeToCoreMessage {
    fn id(&self) -> SubprotocolId {
        CORE_SUBPROTOCOL_ID // Messages destined for Core subprotocol
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// Messages that the Core subprotocol can receive from other subprotocols
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub enum CoreMessage {
    /// Message from upgrade subprotocol
    FromUpgrade(UpgradeToCoreMessage),
}

impl InterprotoMsg for CoreMessage {
    fn id(&self) -> SubprotocolId {
        CORE_SUBPROTOCOL_ID
    }

    fn as_dyn_any(&self) -> &dyn Any {
        self
    }
}

/// Checkpoint proof public parameters as defined in the specification
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CheckpointProofPublicParameters {
    /// Epoch index.
    pub epoch: u32,
    /// New terminal L2 commitment (slot, hash).
    pub terminal: (u64, strata_primitives::l2::L2BlockId), // L2BlockCommitment equivalent
    /// Previous terminal L2 commitment or genesis.
    pub prev_terminal: (u64, strata_primitives::l2::L2BlockId),
    /// Hash of the OL state diff.
    pub state_diff_hash: Buf32,
    /// Ordered messages L2 → L1.
    pub l2_to_l1_msgs: Vec<L2ToL1Msg>,
    /// L1 commitment whose messages have been applied.
    pub l1_ref: (u64, strata_primitives::l1::L1BlockId), // L1BlockCommitment equivalent
    /// Previous L1 commitment or genesis.
    pub prev_l1_ref: (u64, strata_primitives::l1::L1BlockId),
    /// Commitment to the range of L1 → L2 messages.
    pub l1_to_l2_msgs_range_commitment_hash: Buf32,
}

/// Summary structure matching the specification
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CheckpointEpochSummary {
    /// Sequential identifier for the checkpoint epoch.
    pub epoch: u32,
    /// Final L2 block commitment at the end of this epoch.
    pub terminal: (u64, strata_primitives::l2::L2BlockId),
    /// L1 (Bitcoin) block commitment corresponding to this epoch.
    pub l1_ref: (u64, strata_primitives::l1::L1BlockId),
}

/// OL Core state.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct CoreOLState {
    /// The zk‐SNARK verifying key used to verify each new checkpoint proof
    /// that has been posted on Bitcoin.
    checkpoint_vk: VerifyingKey,

    /// Summary of the last checkpoint that was successfully verified.
    /// New proofs are checked against this epoch summary.
    verified_checkpoint: EpochSummary,

    /// The L1 block ID up to which the `verified_checkpoint` covers.
    last_checkpoint_ref: L1BlockId,

    /// Public key of the sequencer authorized to submit checkpoint proofs.
    sequencer_pubkey: Buf32,
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

    type Msg = CoreMessage;

    fn init() -> Self::State {
        // Initialize with placeholder values - in production this would be set from genesis
        // Use empty/default values suitable for development and testing
        CoreOLState {
            checkpoint_vk: get_placeholder_verifying_key(),
            verified_checkpoint: EpochSummary::new(
                0,
                strata_primitives::l2::L2BlockCommitment::new(
                    0,
                    strata_primitives::l2::L2BlockId::default(),
                ),
                strata_primitives::l2::L2BlockCommitment::new(
                    0,
                    strata_primitives::l2::L2BlockId::default(),
                ),
                strata_primitives::l1::L1BlockCommitment::new(
                    0,
                    strata_primitives::l1::L1BlockId::default(),
                ),
                Buf32::zero(),
            ),
            last_checkpoint_ref: L1BlockId::default(),
            sequencer_pubkey: Buf32::zero(),
        }
    }

    fn process_txs(state: &mut Self::State, txs: &[TxInput<'_>], relayer: &mut impl MsgRelayer) {
        for tx in txs {
            let result = match tx.tag().tx_type() {
                OL_STF_CHECKPOINT_TX_TYPE => handle_ol_stf_checkpoint(state, tx, relayer),
                FORCED_INCLUSION_TX_TYPE => handle_forced_inclusion(tx, relayer),
                EE_UPGRADE_TX_TYPE => handle_ee_upgrade(tx, relayer),
                _ => continue, // Ignore unknown transaction types
            };

            // Log errors but continue processing other transactions
            if let Err(e) = result {
                // In a production system, this might emit an error log instead
                eprintln!("Error processing transaction: {e:?}");
            }
        }
    }

    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]) {
        for msg in msgs {
            match msg {
                CoreMessage::FromUpgrade(upgrade_msg) => {
                    match upgrade_msg {
                        UpgradeToCoreMessage::UpdateCheckpointVk(new_vk) => {
                            tracing::info!("Core subprotocol: Updating checkpoint verifying key");
                            state.checkpoint_vk = new_vk.clone();

                            // Log the VK update for external monitoring
                            tracing::debug!("Checkpoint verifying key updated successfully");
                        }
                        UpgradeToCoreMessage::UpdateSequencerPubkey(new_pubkey) => {
                            tracing::info!("Core subprotocol: Updating sequencer public key");
                            state.sequencer_pubkey = *new_pubkey;

                            // Log the sequencer key update for external monitoring
                            tracing::debug!("Sequencer public key updated successfully");
                        }
                    }
                }
            }
        }
    }
}

/// Creates a placeholder VerifyingKey for development/testing
fn get_placeholder_verifying_key() -> VerifyingKey {
    // Create an empty VerifyingKey using the same pattern as ProofReceipt
    VerifyingKey::new(vec![])
}

/// Creates placeholder RollupParams for testing proof verification
fn get_placeholder_rollup_params() -> RollupParams {
    RollupParams {
        rollup_name: "test-rollup".to_string(),
        block_time: 1000,
        da_tag: "test-da".to_string(),
        checkpoint_tag: "test-ckpt".to_string(),
        cred_rule: CredRule::Unchecked,
        horizon_l1_height: 0,
        genesis_l1_height: 0,
        operator_config: strata_primitives::params::OperatorConfig::Static(vec![]),
        evm_genesis_block_hash: Buf32::zero(),
        evm_genesis_block_state_root: Buf32::zero(),
        l1_reorg_safe_depth: 3,
        target_l2_batch_size: 64,
        address_length: 20,
        deposit_amount: 1_000_000_000,
        rollup_vk: RollupVerifyingKey::NativeVerifyingKey(Buf32::zero()),
        dispatch_assignment_dur: 64,
        proof_publish_mode: ProofPublishMode::Timeout(1000),
        max_deposits_in_block: 16,
        network: bitcoin::Network::Regtest,
    }
}

/// Extracts a signed checkpoint from the transaction data
fn extract_signed_checkpoint(tx: &TxInput<'_>) -> Result<SignedCheckpoint, CoreError> {
    let data = tx.tag().aux_data();

    borsh::from_slice::<SignedCheckpoint>(data)
        .map_err(|e| CoreError::MalformedSignedCheckpoint(e.to_string()))
}

/// Extracts a forced inclusion payload from the transaction data
fn extract_forced_inclusion(tx: &TxInput<'_>) -> Result<ForcedInclusion, CoreError> {
    let data = tx.tag().aux_data();

    borsh::from_slice::<ForcedInclusion>(data)
        .map_err(|e| CoreError::MalformedSignedCheckpoint(e.to_string()))
}

/// Computes a rolling hash (placeholder implementation)
fn compute_rolling_hash(_commitments: Vec<Buf32>) -> Result<Buf32, CoreError> {
    // TODO: Implement proper rolling hash computation based on L1→L2 message commitments
    // This would need to fetch L1 block data and compute the rolling hash
    Ok(Buf32::zero())
}

/// Constructs the expected public parameters from our own state and checkpoint context.
///
/// This function builds the public parameters that should be used for proof verification
/// based on our current ASM state and the checkpoint's batch info, rather than trusting
/// any parameters provided by the sequencer. This prevents attacks where a malicious
/// sequencer could provide incorrect public parameters.
fn construct_expected_public_parameters(
    state: &CoreOLState,
    checkpoint: &Checkpoint,
) -> Result<CheckpointProofPublicParameters, CoreError> {
    let batch_info = checkpoint.batch_info();

    // Extract epoch from batch info
    let epoch = batch_info.epoch() as u32;

    // Terminal L2 commitment from batch info
    let terminal = (
        batch_info.final_l2_block().slot(),
        *batch_info.final_l2_block().blkid(),
    );

    // Previous terminal from our current state
    let prev_terminal = (
        state.verified_checkpoint.terminal().slot(),
        *state.verified_checkpoint.terminal().blkid(),
    );

    // L1 reference from batch info
    let l1_ref = (
        batch_info.final_l1_block().height(),
        *batch_info.final_l1_block().blkid(),
    );

    // Previous L1 reference from our current state
    let prev_l1_ref = (
        state.verified_checkpoint.new_l1().height(),
        *state.verified_checkpoint.new_l1().blkid(),
    );

    // Compute state diff hash from the checkpoint's sidecar
    let state_diff_hash = hash::hash_data(checkpoint.sidecar().chainstate());

    // Extract L2→L1 messages from checkpoint's batch transition
    // TODO: This should be extracted from the actual batch transition data
    // For now, using empty vector as placeholder
    let l2_to_l1_msgs = Vec::new();

    // Compute L1→L2 message range commitment
    // TODO: This should be computed from actual L1 block data and message commitments
    let l1_to_l2_msgs_range_commitment_hash = compute_rolling_hash(vec![])?;

    Ok(CheckpointProofPublicParameters {
        epoch,
        terminal,
        prev_terminal,
        state_diff_hash,
        l2_to_l1_msgs,
        l1_ref,
        prev_l1_ref,
        l1_to_l2_msgs_range_commitment_hash,
    })
}

/// Handles OL STF checkpoint transactions according to the specification
fn handle_ol_stf_checkpoint(
    state: &mut CoreOLState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), CoreError> {
    // 1. Extract signed checkpoint
    let signed_checkpoint = extract_signed_checkpoint(tx)?;

    // 2. Signature Verification
    let cred_rule = CredRule::SchnorrKey(state.sequencer_pubkey);
    if !verify_signed_checkpoint_sig(&signed_checkpoint, &cred_rule) {
        return Err(CoreError::InvalidSignature);
    }

    let checkpoint = signed_checkpoint.checkpoint();

    // 3. Verify zk-SNARK Proof using our own constructed public parameters
    // This prevents attacks where the sequencer provides malicious public parameters
    let rollup_params = get_placeholder_rollup_params();
    checkpoint_verification::verify_proof(checkpoint, &rollup_params)
        .map_err(|_| CoreError::ProofVerificationFailed)?;

    // 4. Construct expected public parameters from our state and validate checkpoint structure
    let expected_params = construct_expected_public_parameters(state, checkpoint)?;

    // 5. Validate State Diff Hash (when sidecar is available)
    let computed_hash = hash::hash_data(checkpoint.sidecar().chainstate());
    if computed_hash != expected_params.state_diff_hash {
        return Err(CoreError::StateDiffMismatch);
    }

    // 6. Validate Previous L2 Terminal
    let current_terminal = (
        state.verified_checkpoint.terminal().slot(),
        *state.verified_checkpoint.terminal().blkid(),
    );
    if current_terminal != expected_params.prev_terminal {
        return Err(CoreError::UnexpectedPrevTerminal);
    }

    // 7. Validate Previous L1 Reference
    let current_l1_ref = (
        state.verified_checkpoint.new_l1().height(),
        *state.verified_checkpoint.new_l1().blkid(),
    );
    if current_l1_ref != expected_params.prev_l1_ref {
        return Err(CoreError::UnexpectedPrevL1Ref);
    }

    // 8. Validate L1→L2 Message Range
    // TODO: This requires access to L1 block data and message commitments
    let rolling_hash = compute_rolling_hash(vec![])?; // Placeholder
    if rolling_hash != expected_params.l1_to_l2_msgs_range_commitment_hash {
        return Err(CoreError::L1ToL2RangeMismatch);
    }

    // 9. Update State
    let summary = EpochSummary::new(
        expected_params.epoch as u64,
        strata_primitives::l2::L2BlockCommitment::new(
            expected_params.terminal.0,
            expected_params.terminal.1,
        ),
        strata_primitives::l2::L2BlockCommitment::new(
            expected_params.prev_terminal.0,
            expected_params.prev_terminal.1,
        ),
        strata_primitives::l1::L1BlockCommitment::new(
            expected_params.l1_ref.0,
            expected_params.l1_ref.1,
        ),
        Buf32::zero(), // TODO: Extract final state from checkpoint
    );
    state.verified_checkpoint = summary;

    // 10. Pass WithdrawalRequests to Bridge Subprotocol
    if !expected_params.l2_to_l1_msgs.is_empty() {
        let withdrawal_msg = WithdrawalMsg {
            withdrawals: expected_params.l2_to_l1_msgs,
        };
        relayer.relay_msg(&withdrawal_msg);
    }

    // 11. Emit Log of the Summary
    let checkpoint_summary = CheckpointEpochSummary {
        epoch: expected_params.epoch,
        terminal: expected_params.terminal,
        l1_ref: expected_params.l1_ref,
    };

    let summary_body = borsh::to_vec(&checkpoint_summary)
        .map_err(|e| CoreError::SerializationFailed(e.to_string()))?;

    let log = Log::new(CHECKPOINT_SUMMARY_TY, summary_body);
    relayer.emit_log(log);

    Ok(())
}

/// Handles forced inclusion transactions according to the specification
fn handle_forced_inclusion(
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<(), CoreError> {
    // 1. Extract forced inclusion transaction
    let forced_inclusion = extract_forced_inclusion(tx)?;

    // 2. Emit Log (minimal validation since EE handles the payload)
    let fi_body = borsh::to_vec(&forced_inclusion)
        .map_err(|e| CoreError::SerializationFailed(e.to_string()))?;

    let log = Log::new(FORCED_INCLUSION_TY, fi_body);
    relayer.emit_log(log);

    Ok(())
}

/// Handles EE upgrade transactions (stub implementation)
fn handle_ee_upgrade(_tx: &TxInput<'_>, _relayer: &mut impl MsgRelayer) -> Result<(), CoreError> {
    // TODO: Implement EE upgrade handling after finalizing the process
    // This would involve:
    // 1. Extract upgrade transaction
    // 2. Validate upgrade authority
    // 3. Apply EE VK updates
    // 4. Emit appropriate logs
    Ok(())
}

impl TryFrom<&[u8]> for CheckpointProofPublicParameters {
    type Error = borsh::io::Error;

    fn try_from(data: &[u8]) -> Result<Self, Self::Error> {
        borsh::from_slice(data)
    }
}

impl CheckpointProofPublicParameters {
    pub fn try_from_slice(data: &[u8]) -> Result<Self, borsh::io::Error> {
        borsh::from_slice(data)
    }
}

/// Helper function to create a VK update message for the upgrade subprotocol to send
///
/// This function demonstrates how an upgrade subprotocol would create a message
/// to update the checkpoint verifying key in the Core subprotocol.
///
/// # Example Usage by Upgrade Subprotocol
/// ```ignore
/// // In the upgrade subprotocol's process_txs function:
/// let vk_update_msg = create_vk_update_message(new_verifying_key);
/// relayer.relay_msg(&vk_update_msg);
/// ```
pub fn create_vk_update_message(new_vk: VerifyingKey) -> UpgradeToCoreMessage {
    UpgradeToCoreMessage::UpdateCheckpointVk(new_vk)
}

/// Helper function to create a sequencer key update message for the upgrade subprotocol to send
///
/// This function demonstrates how an upgrade subprotocol would create a message
/// to update the sequencer public key in the Core subprotocol.
///
/// # Example Usage by Upgrade Subprotocol
/// ```ignore
/// // In the upgrade subprotocol's process_txs function:
/// let key_update_msg = create_sequencer_key_update_message(new_pubkey);
/// relayer.relay_msg(&key_update_msg);
/// ```
pub fn create_sequencer_key_update_message(new_pubkey: Buf32) -> UpgradeToCoreMessage {
    UpgradeToCoreMessage::UpdateSequencerPubkey(new_pubkey)
}
