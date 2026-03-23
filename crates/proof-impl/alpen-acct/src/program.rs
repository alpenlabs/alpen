use alloy_genesis::Genesis;
use rkyv::rancor::Error as RkyvError;
use ssz::Decode;
use strata_ee_acct_runtime::EePrivateInput;
use strata_predicate::PredicateKey;
use strata_snark_acct_runtime::PrivateInput as UpdatePrivateInput;
use strata_snark_acct_types::UpdateProofPubParams;
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmInputError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::NativeHost;

use crate::process_ee_acct_update;

/// Host-side input for the EE account update proof.
#[derive(Debug)]
pub struct EeAcctProofInput {
    pub genesis: Genesis,
    pub chunk_predicate_key: PredicateKey,
    pub ee_private_input: EePrivateInput,
    pub update_private_input: UpdatePrivateInput,
}

#[derive(Debug)]
pub struct EeAcctProgram {
    chunk_predicate_key: PredicateKey,
}

impl EeAcctProgram {
    pub fn new(chunk_predicate_key: PredicateKey) -> Self {
        Self {
            chunk_predicate_key,
        }
    }
}

impl ZkVmProgram for EeAcctProgram {
    type Input = EeAcctProofInput;
    type Output = UpdateProofPubParams;

    fn name() -> String {
        "EVM EE Account".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut builder = B::new();
        builder.write_serde(&input.genesis)?;

        let ee_rkyv_bytes = rkyv::to_bytes::<RkyvError>(&input.ee_private_input)
            .map_err(|e| ZkVmInputError::InputBuild(e.to_string()))?;
        builder.write_buf(&ee_rkyv_bytes)?;

        let upd_rkyv_bytes = rkyv::to_bytes::<RkyvError>(&input.update_private_input)
            .map_err(|e| ZkVmInputError::InputBuild(e.to_string()))?;
        builder.write_buf(&upd_rkyv_bytes)?;

        builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        UpdateProofPubParams::from_ssz_bytes(public_values.as_bytes())
            .map_err(|e| ZkVmError::Other(e.to_string()))
    }
}

impl EeAcctProgram {
    pub fn native_host(&self) -> NativeHost {
        let key = self.chunk_predicate_key.clone();
        NativeHost::new(move |zkvm| process_ee_acct_update(zkvm, &key))
    }

    /// Executes the account proof program using the native host for testing.
    pub fn execute(
        &self,
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        let host = self.native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}
