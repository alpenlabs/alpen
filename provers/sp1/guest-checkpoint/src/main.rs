#![no_main]
zkaleido_sp1_guest_env::entrypoint!(main);

use ssz::Decode;
use strata_ol_params::OLRuntimeParams;
use strata_proofimpl_checkpoint::process_ol_stf;
use zkaleido_sp1_guest_env::Sp1ZkVmEnv;

mod runtime_params;

fn main() {
    let runtime_params =
        OLRuntimeParams::from_ssz_bytes(runtime_params::CHECKPOINT_RUNTIME_PARAMS_SSZ)
            .expect("embedded checkpoint runtime params must decode");
    process_ol_stf(&Sp1ZkVmEnv, runtime_params)
}
