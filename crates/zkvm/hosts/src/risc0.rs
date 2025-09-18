use std::sync::LazyLock;

use strata_primitives::proof::ProofContext;
#[cfg(feature = "risc0-builder")]
use strata_risc0_guest_builder::{
    GUEST_RISC0_BTC_BLOCKSPACE_ELF, GUEST_RISC0_CHECKPOINT_ELF, GUEST_RISC0_CL_STF_ELF,
    GUEST_RISC0_EVM_EE_STF_ELF,
};
use zkaleido_risc0_host::Risc0Host;

// Base path for ELF files
pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| std::env::var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/risc0".to_string()));

macro_rules! define_host {
    ($host_name:ident, $guest_const:ident, $elf_file:expr) => {
        #[cfg(feature = "risc0-builder")]
        pub static $host_name: LazyLock<Risc0Host> =
            LazyLock::new(|| Risc0Host::init($guest_const));

        #[cfg(not(feature = "risc0-builder"))]
        pub static $host_name: LazyLock<Risc0Host> = LazyLock::new(|| {
            let elf_path = format!("{}/{}", *ELF_BASE_PATH, $elf_file);
            let elf = std::fs::read(&elf_path)
                .expect(&format!("Failed to read ELF file from {}", elf_path));
            Risc0Host::init(&elf)
        });
    };
}

// Define hosts using the macro
define_host!(
    BTC_BLOCKSPACE_HOST,
    GUEST_RISC0_BTC_BLOCKSPACE_ELF,
    "guest-btc-blockspace.elf"
);
define_host!(
    EVM_EE_STF_HOST,
    GUEST_RISC0_EVM_EE_STF_ELF,
    "guest-evm-ee-stf.elf"
);
define_host!(CL_STF_HOST, GUEST_RISC0_CL_STF_ELF, "guest-cl-stf.elf");
define_host!(
    CHECKPOINT_HOST,
    GUEST_RISC0_CHECKPOINT_ELF,
    "guest-checkpoint.elf"
);

/// Returns a reference to the appropriate [`Risc0Host`] instance based on the given
/// [`ProofContext`].
///
/// This function maps the [`ProofContext`] variant to its corresponding static [`Risc0Host`]
/// instance, allowing for efficient host selection for different proof types.
pub fn get_host(id: &ProofContext) -> &'static Risc0Host {
    match id {
        ProofContext::EvmEeStf(..) => &EVM_EE_STF_HOST,
        ProofContext::ClStf(..) => &CL_STF_HOST,
        ProofContext::Checkpoint(..) => &CHECKPOINT_HOST,
    }
}
