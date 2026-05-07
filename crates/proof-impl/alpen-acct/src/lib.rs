//! EE account update proof implementation wrapping `ee-acct-runtime` with zkaleido proof IO.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_ee_acct_runtime::{ArchivedDaWitness, ArchivedEePrivateInput};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_predicate::PredicateKey;
use strata_snark_acct_runtime::ArchivedPrivateInput as ArchivedUpdatePrivateInput;
use strata_snark_acct_types::UpdateProofPubParams;
use zkaleido::ZkVmEnvSerde;

mod da_inclusion;
mod da_verify;
mod program;

pub use da_inclusion::{
    DaInclusionError, compute_btc_merkle_root, verify_coinbase_inclusion, verify_header_chain,
    verify_wtxid_inclusion,
};
pub use da_verify::{DaVerificationError, bind_da_witness_to_ledger_refs, verify_da_witness};
pub use program::{EeAcctProgram, EeAcctProofInput};

/// Guest entry point for EE account update proof generation.
///
/// Reads a genesis config and three rkyv-serialized private inputs (EE,
/// update, and DA witness) from the zkVM, verifies the account update
/// using the EVM execution environment, and commits the pre-encoded
/// `UpdateProofPubParams` SSZ bytes as public output.
///
/// The `chunk_predicate_key` is a compile-time constant provided by the
/// guest binary, identifying the predicate used to verify chunk proofs.
///
/// The DA-correctness checks layered on top of update verification:
///
/// - reassemble the published `DaBlob` from reveal envelope payloads and bind its `batch_id` to the
///   chunk transitions under proof;
/// - verify each reveal-tx wtxid is included in an L1 block whose header chains up to the public
///   `l1_block_hash`;
/// - bind `da_witness.l1_block_hash` to the highest-idx `LedgerRefs` claim so the OL canonicality
///   check anchors to the same L1 tip as the in-proof inclusion checks.
///
/// State-diff consistency (`apply_state_diff(pre_state, blob.state_diff)`
/// matching the chunk-aggregated post-state root) is **not** checked here;
/// see `verify_da_witness` for the deferred-work note.
pub fn process_ee_acct_update(zkvm: &impl ZkVmEnvSerde, chunk_predicate_key: &PredicateKey) {
    let genesis: Genesis = zkvm.read_serde();
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().unwrap());

    let ee_buf = zkvm.read_buf();
    let ee_input: &ArchivedEePrivateInput =
        rkyv::access::<ArchivedEePrivateInput, RkyvError>(&ee_buf)
            .expect("failed to access rkyv EE archive");

    let upd_buf = zkvm.read_buf();
    let upd_input: &ArchivedUpdatePrivateInput =
        rkyv::access::<ArchivedUpdatePrivateInput, RkyvError>(&upd_buf)
            .expect("failed to access rkyv update archive");

    let da_buf = zkvm.read_buf();
    let da_witness: &ArchivedDaWitness = rkyv::access::<ArchivedDaWitness, RkyvError>(&da_buf)
        .expect("failed to access rkyv DA witness archive");

    let ee = EvmExecutionEnvironment::new(chain_spec);

    strata_ee_acct_runtime::verify_and_process_update(
        &ee,
        chunk_predicate_key,
        ee_input,
        upd_input,
    )
    .expect("account update verification failed");

    // Reassemble the DaBlob, bind its batch_id to the chunks under proof,
    // and verify wtxid + coinbase Merkle inclusion against the public
    // l1_block_hash for every DA block.
    let _da_blob = verify_da_witness(ee_input, da_witness).expect("DA witness verification failed");

    // Bind the in-proof L1 anchor (`da_witness.l1_block_hash`) to the
    // OL-facing anchor (highest-idx `LedgerRefs` entry) so a host can't
    // pass mismatched tips between the inclusion checks and OL
    // canonicality.
    let pub_params = UpdateProofPubParams::from_ssz_bytes(upd_input.update_pub_params_ssz())
        .expect("UpdateProofPubParams must be valid SSZ");
    bind_da_witness_to_ledger_refs(da_witness, pub_params.ledger_refs())
        .expect("DA witness tip and LedgerRefs tip mismatch");

    // Pass through the pre-encoded SSZ bytes directly (zero-copy).
    zkvm.commit_buf(upd_input.update_pub_params_ssz());
}
