use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_bridge_types::WithdrawalIntent;
use strata_crypto::{hash, schnorr::verify_schnorr_sig};
use strata_identifiers::{Buf32, Buf64, CredRule};
use zkaleido::{Proof, ProofReceipt, PublicValues};

use super::batch::BatchInfo;

/// Consolidates all the information that the checkpoint is committing to, signing and proving.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
    DeriveEncode,
    DeriveDecode,
)]
pub struct CheckpointCommitment {
    /// Information regarding the current batches of l1 and l2 blocks along with epoch.
    /// This is verified by the proof
    batch_info: BatchInfo,
}

/// SSZ-friendly representation of [`CheckpointCommitment`].
#[derive(DeriveEncode, DeriveDecode)]
struct CheckpointSsz {
    /// The commitment to the batch of L1 and L2 blocks.
    commitment: CheckpointCommitment,

    /// The proof for this checkpoint.
    proof: Vec<u8>,

    /// The sidecar for this checkpoint.
    sidecar: CheckpointSidecar,
}

/// SSZ-friendly representation of `CheckpointSidecarWithdrawals`.
#[derive(DeriveEncode, DeriveDecode)]
struct CheckpointSidecarWithdrawalsSsz {
    /// The withdrawal intents for this checkpoint.
    withdrawal_intents: Vec<WithdrawalIntent>,
}

/// Consolidates all information required to describe and verify a batch checkpoint.
/// This includes metadata about the batch, the state transitions, checkpoint base state,
/// and the proof itself. The proof verifies that the `transition` is valid.
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct Checkpoint {
    /// Data that this checkpoint is committing to
    commitment: CheckpointCommitment,

    /// Proof for this checkpoint obtained from prover manager.
    proof: Proof,

    /// Additional data we post along with the checkpoint for usability.
    sidecar: CheckpointSidecar,
}

impl Checkpoint {
    pub fn new(batch_info: BatchInfo, proof: Proof, sidecar: CheckpointSidecar) -> Self {
        Self {
            commitment: CheckpointCommitment { batch_info },
            proof,
            sidecar,
        }
    }

    pub fn batch_info(&self) -> &BatchInfo {
        &self.commitment.batch_info
    }

    pub fn commitment(&self) -> &CheckpointCommitment {
        &self.commitment
    }

    pub fn proof(&self) -> &Proof {
        &self.proof
    }

    pub fn set_proof(&mut self, proof: Proof) {
        self.proof = proof
    }

    // #[deprecated(note = "use `checkpoint_verification::construct_receipt`")]
    // TODO: commented for now
    // understand the rationale for making it deprecated
    pub fn construct_receipt(&self) -> ProofReceipt {
        let proof = self.proof().clone();
        let output = self.batch_info().as_ssz_bytes();
        let public_values = PublicValues::new(output);
        ProofReceipt::new(proof, public_values)
    }

    pub fn hash(&self) -> Buf32 {
        // FIXME make this more structured and use incremental hashing

        let mut buf = vec![];
        let batch_serialized = self.commitment.batch_info.as_ssz_bytes();

        buf.extend(&batch_serialized);
        buf.extend(self.proof.as_bytes());

        hash::raw(&buf)
    }

    pub fn sidecar(&self) -> &CheckpointSidecar {
        &self.sidecar
    }
}

impl Encode for Checkpoint {
    fn is_ssz_fixed_len() -> bool {
        <CheckpointSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <CheckpointSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        CheckpointSsz {
            commitment: self.commitment.clone(),
            proof: self.proof.as_bytes().to_vec(),
            sidecar: self.sidecar.clone(),
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        CheckpointSsz {
            commitment: self.commitment.clone(),
            proof: self.proof.as_bytes().to_vec(),
            sidecar: self.sidecar.clone(),
        }
        .ssz_bytes_len()
    }
}

impl Decode for Checkpoint {
    fn is_ssz_fixed_len() -> bool {
        <CheckpointSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <CheckpointSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = CheckpointSsz::from_ssz_bytes(bytes)?;
        Ok(Self {
            commitment: value.commitment,
            proof: Proof::new(value.proof),
            sidecar: value.sidecar,
        })
    }
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    BorshSerialize,
    BorshDeserialize,
    Deserialize,
    Serialize,
    DeriveEncode,
    DeriveDecode,
)]
pub struct CheckpointSidecar {
    /// Chainstate at the end of this checkpoint's epoch.
    /// Note: using `Vec<u8>` instead of Chainstate to avoid circular dependency with strata_state
    chainstate: Vec<u8>,
}

impl CheckpointSidecar {
    pub fn new(chainstate: Vec<u8>) -> Self {
        Self { chainstate }
    }

    /// Creates a new [`CheckpointSidecar`] from a vector of withdrawal intents.
    pub fn from_withdrawals(withdrawal_intents: Vec<WithdrawalIntent>) -> Self {
        Self {
            chainstate: CheckpointSidecarWithdrawalsSsz { withdrawal_intents }.as_ssz_bytes(),
        }
    }

    /// Returns the chainstate for this checkpoint.
    pub fn chainstate(&self) -> &[u8] {
        &self.chainstate
    }

    /// Returns the [`WithdrawalIntent`]s for this checkpoint.
    pub fn withdrawal_intents(&self) -> Result<Vec<WithdrawalIntent>, DecodeError> {
        if self.chainstate.is_empty() {
            return Ok(Vec::new());
        }

        let decoded = CheckpointSidecarWithdrawalsSsz::from_ssz_bytes(&self.chainstate)?;
        Ok(decoded.withdrawal_intents)
    }
}

#[derive(
    Clone, Debug, BorshDeserialize, BorshSerialize, Arbitrary, PartialEq, Eq, Serialize, Deserialize,
)]
pub struct SignedCheckpoint {
    inner: Checkpoint,
    signature: Buf64,
}

/// SSZ-friendly representation of [`SignedCheckpoint`].
#[derive(DeriveEncode, DeriveDecode)]
struct SignedCheckpointSsz {
    /// The inner checkpoint.
    inner: Checkpoint,

    /// The signature for this checkpoint.
    signature: Buf64,
}

impl SignedCheckpoint {
    pub fn new(inner: Checkpoint, signature: Buf64) -> Self {
        Self { inner, signature }
    }

    pub fn checkpoint(&self) -> &Checkpoint {
        &self.inner
    }

    pub fn signature(&self) -> &Buf64 {
        &self.signature
    }
}

impl Encode for SignedCheckpoint {
    fn is_ssz_fixed_len() -> bool {
        <SignedCheckpointSsz as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SignedCheckpointSsz as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        SignedCheckpointSsz {
            inner: self.inner.clone(),
            signature: self.signature,
        }
        .ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        SignedCheckpointSsz {
            inner: self.inner.clone(),
            signature: self.signature,
        }
        .ssz_bytes_len()
    }
}

impl Decode for SignedCheckpoint {
    fn is_ssz_fixed_len() -> bool {
        <SignedCheckpointSsz as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SignedCheckpointSsz as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value = SignedCheckpointSsz::from_ssz_bytes(bytes)?;
        Ok(Self {
            inner: value.inner,
            signature: value.signature,
        })
    }
}

impl From<SignedCheckpoint> for Checkpoint {
    fn from(value: SignedCheckpoint) -> Self {
        value.inner
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct CommitmentInfo {
    pub blockhash: Buf32,
    pub txid: Buf32,
}

impl CommitmentInfo {
    pub fn new(blockhash: Buf32, txid: Buf32) -> Self {
        Self { blockhash, txid }
    }
}

#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct L1CommittedCheckpoint {
    /// The actual `Checkpoint` data.
    pub checkpoint: Checkpoint,
    /// Its commitment to L1 used to locate/identify the checkpoint in L1.
    pub commitment: CommitmentInfo,
}

impl L1CommittedCheckpoint {
    pub fn new(checkpoint: Checkpoint, commitment: CommitmentInfo) -> Self {
        Self {
            checkpoint,
            commitment,
        }
    }
}

/// Verifies that a signed checkpoint has a proper signature according to rollup
/// params.
// TODO this might want to take a chainstate in the future, but we don't have
// the ability to get that where we call this yet
pub fn verify_signed_checkpoint_sig(
    signed_checkpoint: &SignedCheckpoint,
    cred_rule: &CredRule,
) -> bool {
    let seq_pubkey = match cred_rule {
        CredRule::SchnorrKey(key) => key,

        // In this case we always just assume true.
        CredRule::Unchecked => return true,
    };

    let checkpoint_hash = signed_checkpoint.checkpoint().hash();
    verify_schnorr_sig(signed_checkpoint.signature(), &checkpoint_hash, seq_pubkey)
}
