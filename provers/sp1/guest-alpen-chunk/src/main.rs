#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_alpen_chunk::process_ee_chunk;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

fn main() {
    process_ee_chunk(&Sp1ZkVmEnv)
}
