use std::{
    env::var,
    sync::{Arc, LazyLock},
};

#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder::*;
use tokio::sync::OnceCell;
use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/sp1".to_string()));

macro_rules! define_host {
    ($host_fn:ident, $cell_name:ident, $guest_const:ident, $elf_file:expr) => {
        static $cell_name: OnceCell<Arc<SP1Host>> = OnceCell::const_new();

        /// Lazily initializes the host on first call and returns the shared
        /// instance. Subsequent calls return the cached host and ignore the
        /// `config` argument — callers within a single binary are expected to
        /// pass a consistent config.
        pub async fn $host_fn(config: SP1HostConfig) -> &'static Arc<SP1Host> {
            $cell_name
                .get_or_init(|| async {
                    #[cfg(feature = "sp1-builder")]
                    {
                        Arc::new(SP1Host::init_with_config(&$guest_const, config).await)
                    }
                    #[cfg(not(feature = "sp1-builder"))]
                    {
                        let elf_path = format!("{}/{}", *ELF_BASE_PATH, $elf_file);
                        let elf = std::fs::read(&elf_path).unwrap_or_else(|e| {
                            panic!("failed to read ELF file from {elf_path}: {e}")
                        });
                        Arc::new(SP1Host::init_with_config(&elf, config).await)
                    }
                })
                .await
        }
    };
}

define_host!(
    alpen_chunk_host,
    ALPEN_CHUNK_HOST,
    GUEST_ALPEN_CHUNK_ELF,
    "guest-alpen-chunk.elf"
);
define_host!(
    alpen_acct_host,
    ALPEN_ACCT_HOST,
    GUEST_ALPEN_ACCT_ELF,
    "guest-alpen-acct.elf"
);
