use std::{
    env::var,
    sync::{Arc, LazyLock},
};

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
define_host!(
    CHECKPOINT_HOST,
    GUEST_CHECKPOINT_ELF,
    "guest-checkpoint.elf"
);
define_host!(
    CHECKPOINT_NEW_HOST,
    GUEST_CHECKPOINT_NEW_ELF,
    "guest-checkpoint-new.elf"
);
