use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use zkaleido::{ProofType, PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::{
    process_counterproof_outer, CounterproofInput, CounterproofInputBorsh, CounterproofPublicOutput,
};

/// This is responsible for generating the counter-proof
#[derive(Debug)]
pub struct CounterproofProgram;

impl ZkVmProgram for CounterproofProgram {
    type Input = CounterproofInput;
    type Output = CounterproofPublicOutput;

    fn name() -> String {
        "Counter Proof".to_string()
    }

    fn proof_type() -> ProofType {
        zkaleido::ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();
        let borsh_input: CounterproofInputBorsh = input.clone().into();

        input_builder.write_borsh(&borsh_input)?.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        H::extract_borsh_public_output(public_values)
    }
}

impl CounterproofProgram {
    /// get native host. This can be used for testing
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_counterproof_outer(zkvm);
                }))
                .map_err(|_| ZkVmError::ExecutionError(Self::name()))?;
                Ok(())
            })),
        }
    }

    /// Add this new convenience method
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}

/// get native host. This can be used for testing
pub fn get_native_host() -> NativeHost {
    NativeHost {
        process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
            process_counterproof_outer(zkvm);
            Ok(())
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tracing::debug;
    use zkaleido::ZkVmProgram;

    use super::*;
    use crate::{
        test_utils::mock_transaction::{derive_master_pubkey, MockTransactionBuilder},
        CounterproofMode,
    };

    fn run_counterproof_test(mode: CounterproofMode) {
        let bridge_proof_master_key_bytes = [0x01; 32];
        let deposit_index = 32;
        let op_return_data = vec![0x42u8; 292];

        let (mock_tx, mock_prevouts) = MockTransactionBuilder::new()
            .master_key(bridge_proof_master_key_bytes)
            .deposit_index(deposit_index)
            .op_return_data(op_return_data)
            .build();

        let counter_proof_input = CounterproofInput {
            bridge_proof_master_key: derive_master_pubkey(&bridge_proof_master_key_bytes),
            deposit_index,
            bridge_proof_tx: mock_tx,
            bridge_proof_prevouts: Arc::from(mock_prevouts),
            mode,
        };

        let host = get_native_host();
        match CounterproofProgram::prove(&counter_proof_input, &host) {
            Ok(receipt) => debug!(?receipt, "received counter-proof receipt from native host"),
            Err(e) => debug!(?e, "counter-proof failed as expected with dummy data"),
        }
    }

    #[test]
    fn test_invalid_bridge_proof_mode() {
        run_counterproof_test(CounterproofMode::InvalidBridgeProof);
    }

    #[test]
    fn test_heavier_chain_mode() {
        run_counterproof_test(CounterproofMode::HeavierChain(vec![]));
    }
}
