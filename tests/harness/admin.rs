//! Admin subprotocol test utilities
//!
//! Provides ergonomic helpers for testing admin subprotocol transactions.
//!
//! # Example
//!
//! ```ignore
//! use harness::test_harness::AsmTestHarnessBuilder;
//! use harness::admin::{create_test_admin_setup, sequencer_update, AdminExt};
//!
//! let (admin_params, mut ctx) = create_test_admin_setup(2);
//! let harness = AsmTestHarnessBuilder::default()
//!     .with_admin_params(admin_params)
//!     .build()
//!     .await?;
//! harness.submit_admin_action(&mut ctx, sequencer_update([1u8; 32])).await?;
//! let state = harness.admin_state()?;
//! ```

use std::{collections::HashMap, future::Future, num::NonZero, time::Duration};

use bitcoin::{
    secp256k1::{PublicKey, Secp256k1, SecretKey},
    BlockHash,
};
use strata_asm_common::{AnchorState, Subprotocol};
use strata_asm_params::{AdministrationSubprotoParams, Role};
use strata_asm_proto_administration::{AdministrationSubprotoState, AdministrationSubprotocol};
use strata_asm_txs_admin::{
    actions::{
        updates::{
            multisig::MultisigUpdate,
            operator::OperatorSetUpdate,
            predicate::{PredicateUpdate, ProofType},
            seq::SequencerUpdate,
        },
        CancelAction, MultisigAction, UpdateAction,
    },
    parser::SignedPayload,
    test_utils::create_signature_set,
};
use strata_crypto::{
    keys::compressed::CompressedPublicKey,
    threshold_signature::{ThresholdConfig, ThresholdConfigUpdate},
};
use strata_predicate::PredicateKey;
use strata_primitives::buf::Buf32;

use super::test_harness::AsmTestHarness;

/// Admin subprotocol ID per SPS-50.
pub const SUBPROTOCOL_ID: u8 = 0;

/// Extension trait for admin subprotocol operations on the test harness.
///
/// This trait provides admin-specific convenience methods while keeping
/// the core harness infrastructure-focused.
pub trait AdminExt {
    /// Get admin subprotocol state.
    fn admin_state(&self) -> anyhow::Result<AdministrationSubprotoState>;

    /// Submit an admin action: sign, build tx, submit, mine, and wait.
    fn submit_admin_action(
        &self,
        ctx: &mut AdminContext,
        action: MultisigAction,
    ) -> impl Future<Output = anyhow::Result<BlockHash>>;

    /// Submit an admin action with a specific sequence number (for replay testing).
    fn submit_admin_action_with_seqno(
        &self,
        ctx: &AdminContext,
        action: MultisigAction,
        seqno: u64,
    ) -> impl Future<Output = anyhow::Result<BlockHash>>;
}

/// Context for signing admin transactions.
///
/// Tracks sequence numbers per role and provides signing operations for admin actions.
/// Each role's sequence number auto-increments after each successful sign operation.
#[derive(Debug)]
pub struct AdminContext {
    privkeys: Vec<SecretKey>,
    signer_indices: Vec<u8>,
    seqnos: HashMap<Role, u64>,
}

impl AdminContext {
    /// Creates an admin context with the given signing keys.
    pub fn new(privkeys: Vec<SecretKey>, signer_indices: Vec<u8>) -> Self {
        Self {
            privkeys,
            signer_indices,
            seqnos: HashMap::new(),
        }
    }

    /// Sign an action and return (serialized_payload, tx_type).
    ///
    /// Auto-increments the appropriate role's sequence number after signing.
    pub fn sign(&mut self, action: MultisigAction) -> (Vec<u8>, u8) {
        let role = Self::role_for_action(&action);
        let seqno = *self.seqnos.entry(role).or_insert(0);
        let result = self.sign_impl(&action, seqno);
        *self.seqnos.get_mut(&role).unwrap() += 1;
        result
    }

    /// Sign an action with a specific sequence number (for replay attack testing).
    ///
    /// Does NOT auto-increment the internal sequence number.
    pub fn sign_with_seqno(&self, action: MultisigAction, seqno: u64) -> (Vec<u8>, u8) {
        self.sign_impl(&action, seqno)
    }

    /// Get the private keys (for manual signature construction in tests).
    pub fn privkeys(&self) -> &[SecretKey] {
        &self.privkeys
    }

    /// Get the signer indices (for manual signature construction in tests).
    pub fn signer_indices(&self) -> &[u8] {
        &self.signer_indices
    }

    fn role_for_action(action: &MultisigAction) -> Role {
        match action {
            MultisigAction::Update(update) => update.required_role(),
            // Cancel targets StrataAdministrator (sequencer updates are never queued).
            MultisigAction::Cancel(_) => Role::StrataAdministrator,
        }
    }

    fn sign_impl(&self, action: &MultisigAction, seqno: u64) -> (Vec<u8>, u8) {
        let sighash = action.compute_sighash(seqno);
        let sig_set = create_signature_set(&self.privkeys, &self.signer_indices, sighash);
        let payload = borsh::to_vec(&SignedPayload::new(action.clone(), sig_set))
            .expect("serialization should succeed");
        (payload, action.tx_type())
    }
}

// ============================================================================
// Action Builders
// ============================================================================

/// Create a sequencer update action.
pub fn sequencer_update(key: [u8; 32]) -> MultisigAction {
    MultisigAction::Update(UpdateAction::Sequencer(SequencerUpdate::new(Buf32::from(
        key,
    ))))
}

/// Create an operator set update action.
pub fn operator_set_update(add: Vec<[u8; 32]>, remove: Vec<[u8; 32]>) -> MultisigAction {
    MultisigAction::Update(UpdateAction::OperatorSet(OperatorSetUpdate::new(
        add.into_iter().map(Buf32::from).collect(),
        remove.into_iter().map(Buf32::from).collect(),
    )))
}

/// Create a cancel action for a queued update.
pub fn cancel_update(id: u32) -> MultisigAction {
    MultisigAction::Cancel(CancelAction::new(id))
}

/// Create a multisig config update action.
///
/// This updates the threshold configuration for a specific role (admin or sequencer manager).
pub fn multisig_config_update(
    role: Role,
    add_members: Vec<CompressedPublicKey>,
    remove_members: Vec<CompressedPublicKey>,
    new_threshold: u8,
) -> MultisigAction {
    let config = ThresholdConfigUpdate::new(
        add_members,
        remove_members,
        NonZero::new(new_threshold).expect("threshold must be non-zero"),
    );
    MultisigAction::Update(UpdateAction::Multisig(MultisigUpdate::new(config, role)))
}

/// Create a predicate (verifying key) update action.
///
/// This updates the verification key used for proof verification.
pub fn predicate_update(key: PredicateKey, proof_type: ProofType) -> MultisigAction {
    MultisigAction::Update(UpdateAction::VerifyingKey(PredicateUpdate::new(
        key, proof_type,
    )))
}

// ============================================================================
// Test Setup
// ============================================================================

/// Creates matching admin subprotocol params and signing context.
///
/// Generates a random 1-of-1 [`ThresholdConfig`] keypair for both admin roles, so that
/// signatures produced by the returned [`AdminContext`] pass verification against the
/// returned [`AdministrationSubprotoParams`].
pub fn create_test_admin_setup(
    confirmation_depth: u16,
) -> (AdministrationSubprotoParams, AdminContext) {
    let secp = Secp256k1::new();
    let sk = SecretKey::new(&mut rand::thread_rng());
    let pk = CompressedPublicKey::from(PublicKey::from_secret_key(&secp, &sk));
    let config =
        ThresholdConfig::try_new(vec![pk], NonZero::new(1).unwrap()).expect("valid config");

    let params = AdministrationSubprotoParams {
        strata_administrator: config.clone(),
        strata_sequencer_manager: config,
        confirmation_depth,
    };
    let ctx = AdminContext::new(vec![sk], vec![0]);
    (params, ctx)
}

/// Extract admin subprotocol state from AnchorState.
pub fn extract_admin_state(
    anchor_state: &AnchorState,
) -> anyhow::Result<AdministrationSubprotoState> {
    let section = anchor_state
        .find_section(AdministrationSubprotocol::ID)
        .ok_or_else(|| anyhow::anyhow!("Admin section not found"))?;
    let admin_state = section.try_to_state::<AdministrationSubprotocol>()?;
    Ok(admin_state)
}

impl AdminExt for AsmTestHarness {
    fn admin_state(&self) -> anyhow::Result<AdministrationSubprotoState> {
        let (_, asm_state) = self
            .get_latest_asm_state()?
            .ok_or_else(|| anyhow::anyhow!("No ASM state available"))?;
        extract_admin_state(asm_state.state())
    }

    async fn submit_admin_action(
        &self,
        ctx: &mut AdminContext,
        action: MultisigAction,
    ) -> anyhow::Result<BlockHash> {
        let (payload, tx_type) = ctx.sign(action);
        let target_height = self.get_processed_height()? + 1;
        let tx = self
            .build_envelope_tx(SUBPROTOCOL_ID, tx_type, payload)
            .await?;
        let hash = self.submit_and_mine_tx(&tx).await?;
        self.wait_for_height(target_height, Duration::from_secs(5))
            .await?;
        Ok(hash)
    }

    async fn submit_admin_action_with_seqno(
        &self,
        ctx: &AdminContext,
        action: MultisigAction,
        seqno: u64,
    ) -> anyhow::Result<BlockHash> {
        let (payload, tx_type) = ctx.sign_with_seqno(action, seqno);
        let target_height = self.get_processed_height()? + 1;
        let tx = self
            .build_envelope_tx(SUBPROTOCOL_ID, tx_type, payload)
            .await?;
        let hash = self.submit_and_mine_tx(&tx).await?;
        self.wait_for_height(target_height, Duration::from_secs(5))
            .await?;
        Ok(hash)
    }
}
