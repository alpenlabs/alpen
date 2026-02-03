use arbitrary::Arbitrary;
use rkyv::{
    rancor::{Error as RkyvError, Fallible},
    with::{ArchiveWith, DeserializeWith, SerializeWith},
    Archived, Place, Resolver,
};
use serde::{Deserialize, Serialize};
use strata_crypto::{hash, schnorr::verify_schnorr_sig};
use strata_identifiers::{Buf32, Buf64, CredRule};
use zkaleido::{Proof, ProofReceipt, PublicValues};

use super::{batch::BatchInfo, transition::BatchTransition};

/// Serializer for [`Proof`] as bytes for rkyv.
struct ProofAsBytes;

impl ArchiveWith<Proof> for ProofAsBytes {
    type Archived = Archived<Vec<u8>>;
    type Resolver = Resolver<Vec<u8>>;

    fn resolve_with(field: &Proof, resolver: Self::Resolver, out: Place<Self::Archived>) {
        rkyv::Archive::resolve(&field.as_bytes().to_vec(), resolver, out);
    }
}

impl<S> SerializeWith<Proof, S> for ProofAsBytes
where
    S: Fallible + ?Sized,
    Vec<u8>: rkyv::Serialize<S>,
{
    fn serialize_with(field: &Proof, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        rkyv::Serialize::serialize(&field.as_bytes().to_vec(), serializer)
    }
}

impl<D> DeserializeWith<Archived<Vec<u8>>, Proof, D> for ProofAsBytes
where
    D: Fallible + ?Sized,
    Archived<Vec<u8>>: rkyv::Deserialize<Vec<u8>, D>,
{
    fn deserialize_with(
        field: &Archived<Vec<u8>>,
        deserializer: &mut D,
    ) -> Result<Proof, D::Error> {
        let bytes = rkyv::Deserialize::deserialize(field, deserializer)?;
        Ok(Proof::new(bytes))
    }
}

/// Consolidates all the information that the checkpoint is committing to, signing and proving.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct CheckpointCommitment {
    /// Information regarding the current batches of l1 and l2 blocks along with epoch.
    batch_info: BatchInfo,

    /// Transition information verifiable by the proof
    transition: BatchTransition,
}

/// Consolidates all information required to describe and verify a batch checkpoint.
/// This includes metadata about the batch, the state transitions, checkpoint base state,
/// and the proof itself. The proof verifies that the `transition` is valid.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct Checkpoint {
    /// Data that this checkpoint is committing to
    commitment: CheckpointCommitment,

    /// Proof for this checkpoint obtained from prover manager.
    #[rkyv(with = ProofAsBytes)]
    proof: Proof,

    /// Additional data we post along with the checkpoint for usability.
    sidecar: CheckpointSidecar,
}

impl Checkpoint {
    pub fn new(
        batch_info: BatchInfo,
        transition: BatchTransition,
        proof: Proof,
        sidecar: CheckpointSidecar,
    ) -> Self {
        Self {
            commitment: CheckpointCommitment {
                batch_info,
                transition,
            },
            proof,
            sidecar,
        }
    }

    pub fn batch_info(&self) -> &BatchInfo {
        &self.commitment.batch_info
    }

    pub fn batch_transition(&self) -> &BatchTransition {
        &self.commitment.transition
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
        let output = self.batch_transition();
        let public_values = PublicValues::new(
            rkyv::to_bytes::<RkyvError>(output)
                .expect("checkpoint: proof output")
                .into_vec(),
        );
        ProofReceipt::new(proof, public_values)
    }

    pub fn hash(&self) -> Buf32 {
        // FIXME make this more structured and use incremental hashing

        let mut buf = vec![];
        let batch_serialized = rkyv::to_bytes::<RkyvError>(&self.commitment.batch_info)
            .expect("could not serialize checkpoint info");

        buf.extend(batch_serialized.as_ref());
        buf.extend(self.proof.as_bytes());

        hash::raw(&buf)
    }

    pub fn sidecar(&self) -> &CheckpointSidecar {
        &self.sidecar
    }
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
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

    pub fn chainstate(&self) -> &[u8] {
        &self.chainstate
    }
}

#[derive(
    Clone,
    Debug,
    Arbitrary,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct SignedCheckpoint {
    inner: Checkpoint,
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

impl From<SignedCheckpoint> for Checkpoint {
    fn from(value: SignedCheckpoint) -> Self {
        value.inner
    }
}

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
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
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    Serialize,
    Deserialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
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
