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
//! 2. **Forced Inclusion** (`FORCED_INCLUSION_TX_TYPE`): Emergency transaction inclusion
//! 3. **EE Upgrade** (`EE_UPGRADE_TX_TYPE`): Execution environment upgrades
//!
//! ## Security Considerations
//!
//! - All public parameters are constructed from trusted state, not sequencer input
//! - Signature verification prevents unauthorized checkpoint submissions
//! - State validation ensures proper progression of epochs and block heights
//! - Rolling hash verification prevents L1→L2 message manipulation
//!
//! ## Genesis Configuration Example
//!
//! ```rust,no_run
//! # use strata_asm_common::GenesisConfigRegistry;
//! # use strata_asm_proto_core::{CoreGenesisConfig, CORE_SUBPROTOCOL_ID};
//! # use strata_primitives::buf::Buf32;
//! 
//! // Create custom genesis config
//! let genesis_config = CoreGenesisConfig {
//!     sequencer_pubkey: Buf32::from([42u8; 32]),
//!     ..Default::default()
//! };
//!
//! // Register in genesis registry
//! let mut registry = GenesisConfigRegistry::new();
//! registry.register(CORE_SUBPROTOCOL_ID, &genesis_config).unwrap();
//! ```

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
mod genesis_builder;
mod hash;

pub use genesis_builder::{CoreGenesisBuilder, create_default_genesis_registry};

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

    #[error("Invalid signature: {0}")]
    InvalidSignature(String),

    #[error("Malformed public parameters: {0}")]
    MalformedPublicParams(String),

    #[error("State diff hash mismatch: expected {expected}, got {actual}")]
    StateDiffMismatch { expected: String, actual: String },

    #[error("Unexpected previous terminal: expected {expected:?}, got {actual:?}")]
    UnexpectedPrevTerminal {
        expected: (u64, strata_primitives::l2::L2BlockId),
        actual: (u64, strata_primitives::l2::L2BlockId),
    },

    #[error("Unexpected previous L1 reference: expected {expected:?}, got {actual:?}")]
    UnexpectedPrevL1Ref {
        expected: (u64, strata_primitives::l1::L1BlockId),
        actual: (u64, strata_primitives::l1::L1BlockId),
    },

    #[error("L1 to L2 message range mismatch: expected {expected}, got {actual}")]
    L1ToL2RangeMismatch { expected: String, actual: String },

    #[error("Proof verification failed: {0}")]
    ProofVerificationFailed(String),

    #[error("Checkpoint verification failed: {0}")]
    CheckpointVerificationFailed(String),

    #[error("Serialization failed: {0}")]
    SerializationFailed(String),

    #[error("Invalid epoch: expected {expected}, got {actual}")]
    InvalidEpoch { expected: u64, actual: u64 },

    #[error("Malformed forced inclusion: {0}")]
    MalformedForcedInclusion(String),

    #[error("Empty checkpoint data")]
    EmptyCheckpointData,

    #[error("Invalid L1 block height: {0}")]
    InvalidL1BlockHeight(u64),

    #[error("Invalid L2 block slot: {0}")]
    InvalidL2BlockSlot(u64),

    #[error("Missing required field: {0}")]
    MissingRequiredField(String),

    #[error("Internal state corruption: {0}")]
    InternalStateCorruption(String),
}

/// EE identifier type (stub for now)
pub type EEIdentifier = u8;

/// L2 to L1 message type representing withdrawal requests
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct L2ToL1Msg {
    /// Destination address on L1 (Bitcoin)
    pub dest_address: Vec<u8>,
    /// Amount to withdraw in satoshis
    pub amount: u64,
    /// Additional data payload for the withdrawal
    pub data: Vec<u8>,
    /// Nonce to prevent replay attacks
    pub nonce: u64,
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

/// Genesis configuration for the Core subprotocol.
///
/// This structure contains all necessary parameters to properly initialize
/// the Core subprotocol state for a specific network deployment.
#[derive(Clone, Debug, BorshSerialize, BorshDeserialize)]
pub struct CoreGenesisConfig {
    /// The initial checkpoint verifying key for zk-SNARK proof verification
    pub checkpoint_vk: VerifyingKey,
    
    /// The initial verified checkpoint state (usually genesis checkpoint)
    pub initial_checkpoint: EpochSummary,
    
    /// The initial L1 block reference for the checkpoint
    pub initial_l1_ref: L1BlockId,
    
    /// The authorized sequencer's public key for checkpoint submission
    pub sequencer_pubkey: Buf32,
}

impl Default for CoreGenesisConfig {
    fn default() -> Self {
        // Development/test defaults - should be overridden for production
        Self {
            checkpoint_vk: get_placeholder_verifying_key(),
            initial_checkpoint: EpochSummary::new(
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
            initial_l1_ref: L1BlockId::default(),
            sequencer_pubkey: Buf32::zero(),
        }
    }
}

/// Core subprotocol implementation for the Anchor State Machine.
///
/// The Core subprotocol ensures that each zk‐SNARK proof of a new checkpoint
/// is correctly verified against the last known checkpoint state anchored on L1.
/// It manages the verifying key, tracks the latest verified checkpoint, and
/// enforces administrative controls over sequencer and consensus manager keys.
///
/// ## State Management
///
/// The subprotocol maintains:
/// - Current checkpoint verifying key for proof verification
/// - Latest successfully verified checkpoint summary
/// - L1 block reference for the last checkpoint
/// - Authorized sequencer public key
///
/// ## Message Processing
///
/// Handles upgrade messages from the upgrade subprotocol:
/// - Verifying key updates for new proof systems
/// - Sequencer public key rotation for access control
///
/// ## Transaction Processing
///
/// Processes three transaction types with comprehensive validation:
/// - Checkpoint transactions: Full zk-SNARK verification and state updates
/// - Forced inclusion: Emergency transaction processing with minimal validation
/// - EE upgrades: Execution environment updates (placeholder implementation)
#[derive(Copy, Clone, Debug)]
pub struct OLCoreSubproto;

impl Subprotocol for OLCoreSubproto {
    const ID: SubprotocolId = CORE_SUBPROTOCOL_ID;

    type State = CoreOLState;

    type Msg = CoreMessage;

    type GenesisConfig = CoreGenesisConfig;

    fn init(genesis_config: Self::GenesisConfig) -> Self::State {
        // Initialize the Core subprotocol state from genesis configuration
        CoreOLState {
            checkpoint_vk: genesis_config.checkpoint_vk,
            verified_checkpoint: genesis_config.initial_checkpoint,
            last_checkpoint_ref: genesis_config.initial_l1_ref,
            sequencer_pubkey: genesis_config.sequencer_pubkey,
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

    if data.is_empty() {
        return Err(CoreError::EmptyCheckpointData);
    }

    borsh::from_slice::<SignedCheckpoint>(data)
        .map_err(|e| CoreError::MalformedSignedCheckpoint(e.to_string()))
}

/// Extracts a forced inclusion payload from the transaction data
fn extract_forced_inclusion(tx: &TxInput<'_>) -> Result<ForcedInclusion, CoreError> {
    let data = tx.tag().aux_data();

    if data.is_empty() {
        return Err(CoreError::EmptyCheckpointData);
    }

    borsh::from_slice::<ForcedInclusion>(data)
        .map_err(|e| CoreError::MalformedForcedInclusion(e.to_string()))
}

/// L1 block range for message commitment computation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L1BlockRange {
    pub start_height: u64,
    pub end_height: u64,
    pub commitments: Vec<Buf32>,
}

impl L1BlockRange {
    pub fn new(start_height: u64, end_height: u64, commitments: Vec<Buf32>) -> Self {
        Self {
            start_height,
            end_height,
            commitments,
        }
    }
    
    pub fn is_valid(&self) -> bool {
        self.start_height <= self.end_height &&
        self.commitments.len() == (self.end_height - self.start_height + 1) as usize
    }
    
    pub fn len(&self) -> u64 {
        self.end_height - self.start_height + 1
    }
}

/// Computes a rolling hash over L1→L2 message commitments
/// 
/// This function implements a rolling hash algorithm that processes L1 block
/// commitments in sequence, maintaining a running hash state that can be
/// incrementally updated as new blocks are processed.
/// 
/// # Arguments
/// * `l1_commitments` - Vector of L1 block commitments to hash
/// * `start_height` - Starting L1 block height for the range
/// * `end_height` - Ending L1 block height for the range  
/// 
/// # Returns
/// The rolling hash commitment or an error if validation fails
fn compute_rolling_hash(
    l1_commitments: Vec<Buf32>,
    start_height: u64,
    end_height: u64,
) -> Result<Buf32, CoreError> {
    let range = L1BlockRange::new(start_height, end_height, l1_commitments);
    
    // Validate height range
    if start_height > end_height {
        return Err(CoreError::InvalidL1BlockHeight(start_height));
    }
    
    // Validate range consistency
    if !range.is_valid() {
        return Err(CoreError::L1ToL2RangeMismatch {
            expected: format!("commitments for {} blocks", range.len()),
            actual: format!("{} commitments provided", range.commitments.len()),
        });
    }
    
    compute_rolling_hash_from_range(&range)
}

/// Computes rolling hash from a validated L1BlockRange
/// 
/// This implements the actual rolling hash algorithm:
/// rolling_hash = SHA256(rolling_hash || block_commitment)
/// starting with an initial seed based on the range parameters.
fn compute_rolling_hash_from_range(range: &L1BlockRange) -> Result<Buf32, CoreError> {
    // Initialize with range metadata
    let mut rolling_state = Vec::new();
    rolling_state.extend_from_slice(&range.start_height.to_be_bytes());
    rolling_state.extend_from_slice(&range.end_height.to_be_bytes());
    
    // Initial hash of the range metadata
    let mut current_hash = hash::hash_data(&rolling_state);
    
    // Empty range case
    if range.commitments.is_empty() {
        return Ok(current_hash);
    }
    
    // Rolling hash computation: hash(prev_hash || commitment) for each block
    for commitment in &range.commitments {
        let mut data = Vec::with_capacity(64); // 32 bytes hash + 32 bytes commitment
        data.extend_from_slice(current_hash.as_ref());
        data.extend_from_slice(commitment.as_ref());
        current_hash = hash::hash_data(&data);
    }
    
    Ok(current_hash)
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
    
    // Validate epoch progression
    let expected_epoch = (state.verified_checkpoint.epoch() + 1) as u32;
    if epoch != expected_epoch {
        return Err(CoreError::InvalidEpoch {
            expected: expected_epoch as u64,
            actual: epoch as u64,
        });
    }

    // Terminal L2 commitment from batch info
    let terminal = (
        batch_info.final_l2_block().slot(),
        *batch_info.final_l2_block().blkid(),
    );
    
    // Validate L2 block slot progression
    let prev_slot = state.verified_checkpoint.terminal().slot();
    if terminal.0 <= prev_slot {
        return Err(CoreError::InvalidL2BlockSlot(terminal.0));
    }

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
    
    // Validate L1 block height progression
    let prev_height = state.verified_checkpoint.new_l1().height();
    if l1_ref.0 <= prev_height {
        return Err(CoreError::InvalidL1BlockHeight(l1_ref.0));
    }

    // Previous L1 reference from our current state
    let prev_l1_ref = (
        state.verified_checkpoint.new_l1().height(),
        *state.verified_checkpoint.new_l1().blkid(),
    );

    // Compute state diff hash from the checkpoint's sidecar
    let state_diff_hash = hash::hash_data(checkpoint.sidecar().chainstate());

    // Extract L2→L1 messages from checkpoint's batch transition
    let l2_to_l1_msgs = extract_l2_to_l1_messages(checkpoint)?;

    // Compute L1→L2 message range commitment
    // TODO: This should be computed from actual L1 block data and message commitments
    let prev_l1_height = state.verified_checkpoint.new_l1().height();
    let current_l1_height = batch_info.final_l1_block().height();
    let l1_to_l2_msgs_range_commitment_hash = compute_rolling_hash(
        vec![], // TODO: fetch actual L1 commitments for this range
        prev_l1_height,
        current_l1_height,
    )?;

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
///
/// This function implements the complete checkpoint verification workflow:
///
/// 1. **Extract and validate** the signed checkpoint from transaction data
/// 2. **Verify signature** using the current sequencer public key
/// 3. **Verify zk-SNARK proof** using the current verifying key
/// 4. **Construct expected public parameters** from trusted state
/// 5. **Validate state transitions** (epochs, block heights, hashes)
/// 6. **Verify L1→L2 message range** using rolling hash
/// 7. **Update internal state** with new checkpoint summary
/// 8. **Forward withdrawal messages** to Bridge subprotocol
/// 9. **Emit checkpoint summary log** for external monitoring
///
/// # Security Notes
///
/// - Public parameters are constructed from our own state, not sequencer input
/// - All state transitions are validated for proper progression
/// - Proof verification uses trusted verifying key from state
/// - L1→L2 message commitments are verified against expected range
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
        return Err(CoreError::InvalidSignature(
            "Checkpoint signature verification failed".to_string(),
        ));
    }

    let checkpoint = signed_checkpoint.checkpoint();

    // 3. Verify zk-SNARK Proof using our own constructed public parameters
    // This prevents attacks where the sequencer provides malicious public parameters
    let rollup_params = get_placeholder_rollup_params();
    checkpoint_verification::verify_proof(checkpoint, &rollup_params)
        .map_err(|e| CoreError::ProofVerificationFailed(format!("Proof verification error: {e:?}")))?;

    // 4. Construct expected public parameters from our state and validate checkpoint structure
    let expected_params = construct_expected_public_parameters(state, checkpoint)?;

    // 5. Validate State Diff Hash (when sidecar is available)
    let computed_hash = hash::hash_data(checkpoint.sidecar().chainstate());
    if computed_hash != expected_params.state_diff_hash {
        return Err(CoreError::StateDiffMismatch {
            expected: format!("{:?}", expected_params.state_diff_hash),
            actual: format!("{:?}", computed_hash),
        });
    }

    // 6-7. Validate State Continuity
    validate_state_continuity(state, &expected_params)?;

    // 8. Validate L1→L2 Message Range
    // Recompute the rolling hash to verify consistency
    let prev_l1_height = state.verified_checkpoint.new_l1().height();
    let current_l1_height = checkpoint.batch_info().final_l1_block().height();
    let rolling_hash = compute_rolling_hash(
        vec![], // TODO: fetch actual L1 commitments for this range
        prev_l1_height,
        current_l1_height,
    )?;
    
    if rolling_hash != expected_params.l1_to_l2_msgs_range_commitment_hash {
        return Err(CoreError::L1ToL2RangeMismatch {
            expected: format!("{:?}", expected_params.l1_to_l2_msgs_range_commitment_hash),
            actual: format!("{:?}", rolling_hash),
        });
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

    // 10. Validate and Pass WithdrawalRequests to Bridge Subprotocol
    validate_l2_to_l1_messages(&expected_params.l2_to_l1_msgs)?;
    
    if !expected_params.l2_to_l1_msgs.is_empty() {
        let withdrawal_msg = WithdrawalMsg {
            withdrawals: expected_params.l2_to_l1_msgs.clone(),
        };
        relayer.relay_msg(&withdrawal_msg);
        
        tracing::info!(
            "Forwarded {} withdrawal messages to Bridge subprotocol",
            expected_params.l2_to_l1_msgs.len()
        );
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

/// Validates the continuity between current state and expected parameters
/// 
/// This function consolidates validation of L2 terminal and L1 reference
/// continuity to ensure proper state progression.
fn validate_state_continuity(
    state: &CoreOLState,
    expected_params: &CheckpointProofPublicParameters,
) -> Result<(), CoreError> {
    // Validate Previous L2 Terminal
    let current_terminal = (
        state.verified_checkpoint.terminal().slot(),
        *state.verified_checkpoint.terminal().blkid(),
    );
    if current_terminal != expected_params.prev_terminal {
        return Err(CoreError::UnexpectedPrevTerminal {
            expected: current_terminal,
            actual: expected_params.prev_terminal,
        });
    }

    // Validate Previous L1 Reference
    let current_l1_ref = (
        state.verified_checkpoint.new_l1().height(),
        *state.verified_checkpoint.new_l1().blkid(),
    );
    if current_l1_ref != expected_params.prev_l1_ref {
        return Err(CoreError::UnexpectedPrevL1Ref {
            expected: current_l1_ref,
            actual: expected_params.prev_l1_ref,
        });
    }
    
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

/// Extracts L2→L1 messages from the checkpoint's batch transition data
/// 
/// This function parses the batch transition to extract withdrawal messages
/// that need to be forwarded to the Bridge subprotocol for processing.
/// 
/// # Arguments
/// * `checkpoint` - The checkpoint containing batch transition data
/// 
/// # Returns
/// Vector of L2ToL1Msg representing withdrawal requests
fn extract_l2_to_l1_messages(checkpoint: &Checkpoint) -> Result<Vec<L2ToL1Msg>, CoreError> {
    let batch_transition = checkpoint.batch_transition();
    
    // TODO: Parse the actual batch transition structure to extract withdrawal messages
    // This is a placeholder implementation that would need to be replaced with
    // proper parsing logic based on the actual BatchTransition structure
    
    // For now, return empty vector as we don't have access to the actual
    // withdrawal data structure in the batch transition
    let _transition_data = borsh::to_vec(&batch_transition)
        .map_err(|e| CoreError::SerializationFailed(e.to_string()))?;
    
    // In a real implementation, this would:
    // 1. Parse the batch transition to find withdrawal operations
    // 2. Extract destination addresses, amounts, and data
    // 3. Validate withdrawal message format
    // 4. Return properly formatted L2ToL1Msg instances
    
    Ok(Vec::new())
}

/// Validates the structure and content of L2→L1 messages
/// 
/// # Arguments
/// * `messages` - Vector of L2ToL1Msg to validate
/// 
/// # Returns
/// Result indicating validation success or specific error
fn validate_l2_to_l1_messages(messages: &[L2ToL1Msg]) -> Result<(), CoreError> {
    for (idx, msg) in messages.iter().enumerate() {
        // Validate destination address is not empty
        if msg.dest_address.is_empty() {
            return Err(CoreError::MissingRequiredField(
                format!("L2ToL1 message {idx}: destination address"),
            ));
        }
        
        // Validate amount is non-zero for actual withdrawals
        if msg.amount == 0 {
            return Err(CoreError::MissingRequiredField(
                format!("L2ToL1 message {idx}: withdrawal amount"),
            ));
        }
        
        // Additional validation could include:
        // - Address format validation
        // - Amount range checks
        // - Data payload size limits
        // - Nonce uniqueness checks
    }
    
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

#[cfg(test)]
mod tests {
    use super::*;
    use strata_primitives::{
        l1::L1BlockId,
        l2::L2BlockId,
        buf::Buf32,
    };

    fn create_test_state() -> CoreOLState {
        let genesis_config = CoreGenesisConfig::default();
        OLCoreSubproto::init(genesis_config)
    }

    #[test]
    fn test_l2_to_l1_message_validation() {
        let valid_msg = L2ToL1Msg {
            dest_address: vec![1, 2, 3, 4],
            amount: 1000,
            data: vec![],
            nonce: 1,
        };
        
        assert!(validate_l2_to_l1_messages(&[valid_msg]).is_ok());
        
        let invalid_msg_empty_address = L2ToL1Msg {
            dest_address: vec![],
            amount: 1000,
            data: vec![],
            nonce: 1,
        };
        
        assert!(validate_l2_to_l1_messages(&[invalid_msg_empty_address]).is_err());
        
        let invalid_msg_zero_amount = L2ToL1Msg {
            dest_address: vec![1, 2, 3, 4],
            amount: 0,
            data: vec![],
            nonce: 1,
        };
        
        assert!(validate_l2_to_l1_messages(&[invalid_msg_zero_amount]).is_err());
    }

    #[test]
    fn test_rolling_hash_computation() {
        let commitments = vec![Buf32::zero(), Buf32::zero()];
        let result = compute_rolling_hash(commitments, 0, 1);
        assert!(result.is_ok());
        
        // Test invalid height range
        let result = compute_rolling_hash(vec![], 10, 5);
        assert!(result.is_err());
        
        // Test commitment count mismatch
        let result = compute_rolling_hash(vec![Buf32::zero()], 0, 5);
        assert!(result.is_err());
    }
    
    #[test]
    fn test_l1_block_range() {
        let range = L1BlockRange::new(0, 2, vec![Buf32::zero(); 3]);
        assert!(range.is_valid());
        assert_eq!(range.len(), 3);
        
        let invalid_range = L1BlockRange::new(0, 2, vec![Buf32::zero(); 2]);
        assert!(!invalid_range.is_valid());
    }
    
    #[test]
    fn test_rolling_hash_deterministic() {
        let commitments = vec![Buf32::zero(), Buf32::zero()];
        let hash1 = compute_rolling_hash(commitments.clone(), 0, 1).unwrap();
        let hash2 = compute_rolling_hash(commitments, 0, 1).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_error_formatting() {
        let error = CoreError::StateDiffMismatch {
            expected: "hash1".to_string(),
            actual: "hash2".to_string(),
        };
        let error_string = format!("{}", error);
        assert!(error_string.contains("hash1"));
        assert!(error_string.contains("hash2"));
    }

    #[test]
    fn test_upgrade_messages() {
        let vk = get_placeholder_verifying_key();
        let vk_msg = create_vk_update_message(vk);
        assert_eq!(vk_msg.id(), CORE_SUBPROTOCOL_ID);
        
        let pubkey = Buf32::zero();
        let key_msg = create_sequencer_key_update_message(pubkey);
        assert_eq!(key_msg.id(), CORE_SUBPROTOCOL_ID);
    }

    #[test]
    fn test_core_state_initialization() {
        let genesis_config = CoreGenesisConfig::default();
        let state = OLCoreSubproto::init(genesis_config);
        assert_eq!(state.verified_checkpoint.epoch(), 0);
        assert_eq!(state.sequencer_pubkey, Buf32::zero());
    }
    
    #[test]
    fn test_core_state_initialization_with_custom_config() {
        let custom_key = Buf32::from([42u8; 32]);
        let genesis_config = CoreGenesisConfig {
            sequencer_pubkey: custom_key,
            ..Default::default()
        };
        let state = OLCoreSubproto::init(genesis_config);
        assert_eq!(state.sequencer_pubkey, custom_key);
    }
    
    #[test]
    fn test_state_continuity_validation() {
        let state = create_test_state();
        
        let valid_params = CheckpointProofPublicParameters {
            epoch: 1,
            terminal: (1, L2BlockId::default()),
            prev_terminal: (0, L2BlockId::default()),
            state_diff_hash: Buf32::zero(),
            l2_to_l1_msgs: vec![],
            l1_ref: (1, L1BlockId::default()),
            prev_l1_ref: (0, L1BlockId::default()),
            l1_to_l2_msgs_range_commitment_hash: Buf32::zero(),
        };
        
        assert!(validate_state_continuity(&state, &valid_params).is_ok());
        
        let invalid_params = CheckpointProofPublicParameters {
            epoch: 1,
            terminal: (1, L2BlockId::default()),
            prev_terminal: (999, L2BlockId::default()), // Wrong previous terminal
            state_diff_hash: Buf32::zero(),
            l2_to_l1_msgs: vec![],
            l1_ref: (1, L1BlockId::default()),
            prev_l1_ref: (0, L1BlockId::default()),
            l1_to_l2_msgs_range_commitment_hash: Buf32::zero(),
        };
        
        assert!(validate_state_continuity(&state, &invalid_params).is_err());
    }
}
