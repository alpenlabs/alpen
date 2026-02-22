#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_checkpoint_new::process_ol_stf;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

fn main() {
    process_ol_stf(&Sp1ZkVmEnv)
}
