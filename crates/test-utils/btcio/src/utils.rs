use std::future::Future;

use bitcoin::{
    Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Witness, absolute::LockTime,
    transaction::Version,
};
use tokio::{runtime, task::block_in_place};

/// If we're already in a tokio runtime, we'll block in place. Otherwise, we'll create a new
/// runtime.
pub(crate) fn block_on<T>(fut: impl Future<Output = T>) -> T {
    // Handle case if we're already in an tokio runtime.
    if let Ok(handle) = runtime::Handle::try_current() {
        block_in_place(|| handle.block_on(fut))
    } else {
        // Otherwise create a new runtime.
        let rt = runtime::Runtime::new().expect("Failed to create a new runtime");
        rt.block_on(fut)
    }
}

/// Creates a dummy Bitcoin transaction with the specified number of inputs and outputs.
///
/// The inputs will have null previous outputs and empty script sigs.
/// The outputs will have zero value and empty script pubkeys.
/// The transaction version is set to 2, and lock time to 0.
pub fn create_dummy_tx(num_inputs: usize, num_outputs: usize) -> Transaction {
    let input = (0..num_inputs)
        .map(|_| TxIn {
            previous_output: OutPoint::null(),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    let output = (0..num_outputs)
        .map(|_| TxOut {
            value: Amount::ZERO,
            script_pubkey: ScriptBuf::new(),
        })
        .collect();

    Transaction {
        version: Version(2),
        lock_time: LockTime::ZERO,
        input,
        output,
    }
}
