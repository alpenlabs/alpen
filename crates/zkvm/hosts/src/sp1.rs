use std::sync::LazyLock;

use strata_crypto::proof_vk::ProofContext;
#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder::*;
use zkaleido_sp1_host::SP1Host;

pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| std::env::var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/sp1".to_string()));

macro_rules! define_host {
    ($host_name:ident, $guest_const:ident, $elf_file:expr) => {
        #[cfg(feature = "sp1-builder")]
        pub static $host_name: LazyLock<SP1Host> = LazyLock::new(|| SP1Host::init(&$guest_const));

        #[cfg(not(feature = "sp1-builder"))]
        pub static $host_name: LazyLock<SP1Host> = LazyLock::new(|| {
            let elf_path = format!("{}/{}", *ELF_BASE_PATH, $elf_file);
            let elf = std::fs::read(&elf_path)
                .expect(&format!("Failed to read ELF file from {}", elf_path));
            SP1Host::init(&elf)
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

/// Returns a reference to the appropriate `SP1Host` instance based on the given [`ProofContext`].
///
/// This function maps the `ProofContext` variant to its corresponding static [`SP1Host`]
/// instance, allowing for efficient host selection for different proof types.
pub fn get_host(id: &ProofContext) -> &'static SP1Host {
    match id {
        ProofContext::EvmEeStf(..) => &EVM_EE_STF_HOST,
        ProofContext::ClStf(..) => &CL_STF_HOST,
        ProofContext::Checkpoint(..) => &CHECKPOINT_HOST,
    }
}
