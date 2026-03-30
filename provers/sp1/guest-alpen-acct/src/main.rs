#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

mod vks;

use strata_predicate::PredicateKey;
use strata_proofimpl_alpen_acct::process_ee_acct_update;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

/// Constructs the chunk proof predicate key from the full Groth16 verifying
/// key condition bytes that `build.rs` embeds in `vks.rs`.
#[cfg(feature = "zkvm-verify")]
fn chunk_predicate_key() -> PredicateKey {
    use strata_predicate::PredicateTypeId;

    PredicateKey::new(
        PredicateTypeId::Sp1Groth16,
        vks::GUEST_ALPEN_CHUNK_VK_CONDITION.to_vec(),
    )
}

/// In mock builds, verification is a no-op so `always_accept` suffices.
#[cfg(not(feature = "zkvm-verify"))]
fn chunk_predicate_key() -> PredicateKey {
    PredicateKey::always_accept()
}

fn main() {
    let key = chunk_predicate_key();
    process_ee_acct_update(&Sp1ZkVmEnv, &key)
}
