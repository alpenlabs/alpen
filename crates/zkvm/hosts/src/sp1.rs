use std::{
    env::var,
    sync::{Arc, LazyLock},
};

use strata_primitives::proof::ProofContext;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder::*;
use zkaleido_sp1_host::SP1Host;

pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/sp1".to_string()));

macro_rules! define_host {
    ($host_name:ident, $guest_const:ident, $elf_file:expr) => {
        #[cfg(feature = "sp1-builder")]
        pub static $host_name: LazyLock<Arc<SP1Host>> =
            LazyLock::new(|| Arc::new(SP1Host::init(&$guest_const)));

        #[cfg(not(feature = "sp1-builder"))]
        pub static $host_name: LazyLock<Arc<SP1Host>> = LazyLock::new(|| {
            let elf_path = format!("{}/{}", *ELF_BASE_PATH, $elf_file);
            let elf = std::fs::read(&elf_path)
                .expect(&format!("Failed to read ELF file from {}", elf_path));
            Arc::new(SP1Host::init(&elf))
        });
    };
}

// Define hosts using the macro
define_host!(
    EVM_EE_STF_HOST,
    GUEST_EVM_EE_STF_ELF,
    "guest-evm-ee-stf.elf"
);
define_host!(CL_STF_HOST, GUEST_CL_STF_ELF, "guest-cl-stf.elf");
define_host!(
    CHECKPOINT_HOST,
    GUEST_CHECKPOINT_ELF,
    "guest-checkpoint.elf"
);
define_host!(
    ALPEN_EE_ACCT_HOST,
    GUEST_ALPEN_EE_ACCT_ELF,
    "guest-alpen-ee-acct.elf"
);

/// Returns a cloned Arc to the appropriate `SP1Host` instance based on the given [`ProofContext`].
///
/// This function maps the `ProofContext` variant to its corresponding static [`SP1Host`]
/// instance, allowing for efficient host selection for different proof types. The Arc is
/// cloned cheaply (only incrementing reference count).
pub fn get_host(id: &ProofContext) -> Arc<SP1Host> {
    match id {
        ProofContext::EvmEeStf(..) => Arc::clone(&EVM_EE_STF_HOST),
        ProofContext::ClStf(..) => Arc::clone(&CL_STF_HOST),
        ProofContext::Checkpoint(..) => Arc::clone(&CHECKPOINT_HOST),
        ProofContext::EthEeAcct(..) => Arc::clone(&ALPEN_EE_ACCT_HOST),
    }
}
