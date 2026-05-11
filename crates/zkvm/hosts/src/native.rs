//! Deterministic native zkVM hosts used by local functional tests.

use std::{fmt, sync::Arc};

use k256::schnorr::{
    signature::{Signer, Verifier},
    Signature, SigningKey,
};
use strata_predicate::{PredicateKey, PredicateTypeId};
use zkaleido::{
    AggregationInput, DataFormatError, ExecutionSummary, ProgramId, Proof, ProofMetadata,
    ProofReceipt, ProofReceiptWithMetadata, ProofType, PublicValues, VerifyingKey, ZkVm, ZkVmError,
    ZkVmExecutor, ZkVmHost, ZkVmInputBuilder, ZkVmInputError, ZkVmInputResult, ZkVmOutputExtractor,
    ZkVmProofError, ZkVmProver, ZkVmResult, ZkVmTypedVerifier, ZkVmVkProvider,
};
use zkaleido_native_adapter::NativeMachine;

type ProcessProofFn = dyn Fn(&NativeMachine) -> ZkVmResult<()> + Send + Sync;

const CHECKPOINT_NATIVE_SECRET: [u8; 32] = [0x11; 32];
const ALPEN_CHUNK_NATIVE_SECRET: [u8; 32] = [0x22; 32];
const ALPEN_ACCT_NATIVE_SECRET: [u8; 32] = [0x33; 32];

/// Native proof programs that need stable Schnorr predicate keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProofKind {
    /// OL checkpoint proof.
    Checkpoint,
    /// Alpen EE chunk proof.
    AlpenChunk,
    /// Alpen EE account update proof.
    AlpenAcct,
}

impl NativeProofKind {
    fn secret(self) -> [u8; 32] {
        match self {
            Self::Checkpoint => CHECKPOINT_NATIVE_SECRET,
            Self::AlpenChunk => ALPEN_CHUNK_NATIVE_SECRET,
            Self::AlpenAcct => ALPEN_ACCT_NATIVE_SECRET,
        }
    }

    /// Returns the predicate key that verifies this native proof kind.
    pub fn predicate_key(self) -> PredicateKey {
        predicate_key_for_secret(self.secret())
    }
}

/// Returns the native Schnorr predicate for OL checkpoint proofs.
pub fn checkpoint_predicate_key() -> PredicateKey {
    NativeProofKind::Checkpoint.predicate_key()
}

/// Returns the native Schnorr predicate for Alpen EE chunk proofs.
pub fn alpen_chunk_predicate_key() -> PredicateKey {
    NativeProofKind::AlpenChunk.predicate_key()
}

/// Returns the native Schnorr predicate for Alpen EE account update proofs.
pub fn alpen_acct_predicate_key() -> PredicateKey {
    NativeProofKind::AlpenAcct.predicate_key()
}

fn signing_key_for_secret(secret: [u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&secret).expect("native proof secret key must be valid")
}

fn predicate_key_for_secret(secret: [u8; 32]) -> PredicateKey {
    let signing_key = signing_key_for_secret(secret);
    PredicateKey::new(
        PredicateTypeId::Bip340Schnorr,
        signing_key.verifying_key().to_bytes().to_vec(),
    )
}

/// Native host that signs public values with a stable Schnorr key.
#[derive(Clone)]
pub struct DeterministicNativeHost {
    process_fn: Arc<Box<ProcessProofFn>>,
    signing_key: SigningKey,
}

impl DeterministicNativeHost {
    /// Creates a deterministic native host for the given proof kind.
    pub fn new<F>(process_fn: F, kind: NativeProofKind) -> Self
    where
        F: Fn(&NativeMachine) + Send + Sync + 'static,
    {
        Self::new_fallible(
            move |zkvm| {
                process_fn(zkvm);
                Ok(())
            },
            kind,
        )
    }

    /// Creates a deterministic native host with a fallible process function.
    pub fn new_fallible<F>(process_fn: F, kind: NativeProofKind) -> Self
    where
        F: Fn(&NativeMachine) -> ZkVmResult<()> + Send + Sync + 'static,
    {
        Self {
            process_fn: Arc::new(Box::new(process_fn)),
            signing_key: signing_key_for_secret(kind.secret()),
        }
    }

    /// Returns the predicate key that verifies receipts from this host.
    pub fn predicate_key(&self) -> PredicateKey {
        PredicateKey::new(
            PredicateTypeId::Bip340Schnorr,
            self.signing_key.verifying_key().to_bytes().to_vec(),
        )
    }
}

impl ZkVmHost for DeterministicNativeHost {
    fn zkvm(&self) -> ZkVm {
        ZkVm::Native
    }
}

impl ZkVmExecutor for DeterministicNativeHost {
    type Input<'a> = NativeInputBuilder;

    fn execute<'a>(&self, native_machine: NativeMachine) -> ZkVmResult<ExecutionSummary> {
        (self.process_fn)(&native_machine)?;
        let output = native_machine.state.borrow().output.clone();
        Ok(ExecutionSummary::new(PublicValues::new(output), 0, None))
    }

    fn get_elf(&self) -> &[u8] {
        &[]
    }

    fn save_trace(&self, _trace_name: &str) {}

    fn program_id(&self) -> ProgramId {
        ProgramId(self.signing_key.verifying_key().to_bytes().into())
    }
}

impl ZkVmProver for DeterministicNativeHost {
    type ZkVmProofReceipt = NativeProofReceipt;

    fn prove_inner<'a>(
        &self,
        native_machine: NativeMachine,
        proof_type: ProofType,
    ) -> ZkVmResult<NativeProofReceipt> {
        let execution_result = self.execute(native_machine)?;
        let public_values = execution_result.into_public_values();
        let signature: Signature = self.signing_key.sign(public_values.as_bytes());
        let proof = Proof::new(signature.to_bytes().to_vec());
        let receipt = ProofReceipt::new(proof, public_values);
        let metadata = ProofMetadata::new(
            ZkVm::Native,
            self.program_id(),
            env!("CARGO_PKG_VERSION").to_string(),
            proof_type,
        );
        let receipt = ProofReceiptWithMetadata::new(receipt, metadata);
        Ok(receipt.try_into()?)
    }
}

impl ZkVmTypedVerifier for DeterministicNativeHost {
    type ZkVmProofReceipt = NativeProofReceipt;

    fn verify_inner(&self, proof: &NativeProofReceipt) -> ZkVmResult<()> {
        let receipt: ProofReceiptWithMetadata = proof
            .clone()
            .try_into()
            .map_err(ZkVmError::InvalidProofReceipt)?;
        let signature = Signature::try_from(receipt.receipt().proof().as_bytes())
            .map_err(|e| ZkVmError::ProofVerificationError(format!("invalid signature: {e}")))?;
        self.signing_key
            .verifying_key()
            .verify(receipt.receipt().public_values().as_bytes(), &signature)
            .map_err(|e| {
                ZkVmError::ProofVerificationError(format!("signature verification failed: {e}"))
            })
    }
}

impl ZkVmVkProvider for DeterministicNativeHost {
    fn vk(&self) -> VerifyingKey {
        VerifyingKey::new(self.signing_key.verifying_key().to_bytes().to_vec())
    }
}

impl ZkVmOutputExtractor for DeterministicNativeHost {
    fn extract_serde_public_output<T: serde::Serialize + serde::de::DeserializeOwned>(
        public_values_raw: &PublicValues,
    ) -> ZkVmResult<T> {
        bincode::deserialize(public_values_raw.as_bytes()).map_err(|e| {
            ZkVmError::OutputExtractionError {
                source: DataFormatError::Serde(e.to_string()),
            }
        })
    }
}

impl fmt::Debug for DeterministicNativeHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "deterministic_native")
    }
}

/// Input builder for the deterministic native host.
#[derive(Debug)]
pub struct NativeInputBuilder(NativeMachine);

impl ZkVmInputBuilder<'_> for NativeInputBuilder {
    type Input = NativeMachine;
    type ZkVmProofReceipt = NativeProofReceipt;

    fn new() -> Self {
        Self(NativeMachine::new())
    }

    fn write_buf(&mut self, item: &[u8]) -> ZkVmInputResult<&mut Self> {
        self.0.write_slice(item.to_vec());
        Ok(self)
    }

    fn write_serde<T: serde::Serialize>(&mut self, item: &T) -> ZkVmInputResult<&mut Self> {
        let slice = bincode::serialize(item)
            .map_err(|e| ZkVmInputError::DataFormat(DataFormatError::Serde(e.to_string())))?;
        self.write_buf(&slice)
    }

    fn write_proof(&mut self, item: &AggregationInput) -> ZkVmInputResult<&mut Self> {
        self.write_buf(item.receipt().receipt().public_values().as_bytes())
    }

    fn build(&mut self) -> ZkVmInputResult<Self::Input> {
        Ok(self.0.clone())
    }
}

/// Native proof receipt wrapper.
#[derive(Debug, Clone)]
pub struct NativeProofReceipt(ProofReceiptWithMetadata);

impl TryFrom<ProofReceiptWithMetadata> for NativeProofReceipt {
    type Error = ZkVmProofError;

    fn try_from(value: ProofReceiptWithMetadata) -> Result<Self, Self::Error> {
        Ok(Self(value))
    }
}

impl TryFrom<NativeProofReceipt> for ProofReceiptWithMetadata {
    type Error = ZkVmProofError;

    fn try_from(value: NativeProofReceipt) -> Result<Self, Self::Error> {
        Ok(value.0)
    }
}
