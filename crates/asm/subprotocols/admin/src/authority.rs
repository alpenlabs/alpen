use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_params::Role;
use strata_asm_txs_admin::parser::SignedPayload;
use strata_crypto::threshold_signature::{ThresholdConfig, verify_threshold_signatures};

use crate::error::AdministrationError;

/// Manages threshold signature operations for a given role and key set, with replay protection via
/// a sequence number.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct MultisigAuthority {
    /// The role of this threshold signature authority.
    role: Role,
    /// The public keys of all grant-holders authorized to sign.
    config: ThresholdConfig,
    /// The last sequence number that was successfully executed.
    /// This is used to prevent replay attacks.
    last_seqno: u64,
}

impl MultisigAuthority {
    /// Creates a new authority with `last_seqno` initialized to 0.
    ///
    /// Since `verify_action_signature` requires `payload.seqno > self.last_seqno`, the first
    /// valid payload must have `seqno >= 1`.
    pub fn new(role: Role, config: ThresholdConfig) -> Self {
        Self {
            role,
            config,
            last_seqno: 0,
        }
    }

    /// The role authorized to perform threshold signature operations.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Borrow the current threshold configuration.
    pub fn config(&self) -> &ThresholdConfig {
        &self.config
    }

    /// Mutably borrow the threshold configuration.
    pub(crate) fn config_mut(&mut self) -> &mut ThresholdConfig {
        &mut self.config
    }

    /// Verifies a set of ECDSA signatures against a threshold configuration.
    //
    // This function is intentionally ECDSA-specific as part of the hardware wallet
    // compatibility design (BIP-137 format support). A trait-based abstraction
    // could be added in the future if multiple signature schemes are needed.
    pub fn verify_action_signature(
        &self,
        payload: &SignedPayload,
        max_seqno_gap: u8,
    ) -> Result<(), AdministrationError> {
        if payload.seqno <= self.last_seqno {
            return Err(AdministrationError::InvalidSeqno {
                payload_seqno: payload.seqno,
                last_seqno: self.last_seqno,
            });
        }

        if payload.seqno > self.last_seqno + max_seqno_gap as u64 {
            return Err(AdministrationError::SeqnoGapTooLarge {
                payload_seqno: payload.seqno,
                last_seqno: self.last_seqno,
                max_gap: max_seqno_gap,
            });
        }
        // Compute the msg to sign by combining UpdateAction with sequence no
        let sig_hash = payload.action.compute_sighash(payload.seqno);

        verify_threshold_signatures(
            &self.config,
            payload.signatures.signatures(),
            &sig_hash.into(),
        )?;

        Ok(())
    }

    /// Updates the last executed seqno.
    pub(crate) fn update_last_seqno(&mut self, seqno: u64) {
        self.last_seqno = seqno;
    }

    /// Returns the last successfully executed sequence number.
    pub fn last_seqno(&self) -> u64 {
        self.last_seqno
    }
}
