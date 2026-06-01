#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use strata_proofimpl_evm_ee_stf::process_block_transaction_outer;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

fn main() {
    process_block_transaction_outer(&Sp1ZkVmEnv)
}
