// These two lines are necessary for the program to properly compile.
//
// Under the hood, we wrap your main function with some extra code so that it behaves properly
// inside the zkVM.
#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_eth_ee_acct::process_eth_ee_acct_update;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

fn main() {
    process_eth_ee_acct_update(&Sp1ZkVmEnv)
}
