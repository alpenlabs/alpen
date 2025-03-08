// These two lines are necessary for the program to properly compile.
//
// Under the hood, we wrap your main function with some extra code so that it behaves properly
// inside the zkVM.
#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_cl_stf::process_cl_stf;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

mod vks;

fn main() {
    process_cl_stf(
        &Sp1ZkVmEnv,
        vks::GUEST_EVM_EE_STF_ELF_ID,
        vks::GUEST_BTC_BLOCKSPACE_ELF_ID,
    );
}
