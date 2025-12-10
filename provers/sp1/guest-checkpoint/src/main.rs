// These two lines are necessary for the program to properly compile.
//
// Under the hood, we wrap your main function with some extra code so that it behaves properly
// inside the zkVM.
#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_checkpoint::process_checkpoint_proof;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

mod vks;

fn main() {
    process_checkpoint_proof(&Sp1ZkVmEnv, vks::GUEST_OL_STF_ELF_ID)
}
