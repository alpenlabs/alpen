use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
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

#[cfg(test)]
mod tests {
    use rsp_primitives::genesis::Genesis;
    use ssz::Encode;
    use strata_acct_types::BitcoinAmount;
    use strata_codec::encode_to_vec;
    use strata_ee_acct_runtime::EePrivateInput;
    use strata_ee_acct_types::{EeAccountState, UpdateExtraData};
    use strata_identifiers::Hash;
    use strata_predicate::PredicateKey;
    use strata_snark_acct_runtime::{IInnerState, PrivateInput as UpdatePrivateInput};
    use strata_snark_acct_types::{LedgerRefs, ProofState, UpdateOutputs, UpdateProofPubParams};

    use super::*;

    /// Smoke test: constructs a minimal self-consistent input with zero chunks
    /// and zero messages, and runs through the full native execution pipeline.
    #[test]
    fn test_native_acct_execution_zero_chunks() {
        // Build a minimal EE account state.
        let initial_blkid = Hash::zero();
        let initial_state = EeAccountState::new(
            initial_blkid,
            BitcoinAmount::from_sat(0),
            Vec::new(),
            Vec::new(),
        );
        let state_root = initial_state.compute_state_root();

        // Extra data: tip stays the same, nothing processed.
        let extra_data = UpdateExtraData::new(initial_blkid, 0, 0);
        let extra_data_bytes = encode_to_vec(&extra_data).expect("encode extra data");

        // With zero chunks and no state change, pre == post state root.
        let pub_params = UpdateProofPubParams::new(
            ProofState::new(state_root, 0),
            ProofState::new(state_root, 0),
            vec![],
            LedgerRefs::new_empty(),
            UpdateOutputs::new_empty(),
            extra_data_bytes,
        );

        // Construct private inputs.
        let update_private_input =
            UpdatePrivateInput::new(pub_params, initial_state.as_ssz_bytes(), vec![]);
        let ee_private_input = EePrivateInput::new(vec![], vec![], vec![]);

        // Use Mainnet genesis (valid ChainSpec, not used with zero chunks).
        let genesis = Genesis::Mainnet;
        let predicate_key = PredicateKey::always_accept();

        let proof_input = EeAcctProofInput {
            genesis,
            chunk_predicate_key: predicate_key.clone(),
            ee_private_input,
            update_private_input,
        };

        let program = EeAcctProgram::new(predicate_key);
        let result = program
            .execute(&proof_input)
            .expect("native execution should succeed");

        // Verify output pub params state roots match.
        assert_eq!(result.cur_state().inner_state(), state_root);
        assert_eq!(result.new_state().inner_state(), state_root);
    }
}
