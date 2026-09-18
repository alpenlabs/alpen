use std::{
    env::var,
    fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
};

#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder::CHECKPOINT_ELF_PATH;
use tokio::sync::OnceCell;
use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/sp1".to_string()));

const GUEST_CHECKPOINT_ARTIFACT_MANIFEST_FILE: &str = "guest-checkpoint.artifact-manifest.json";

pub fn checkpoint_runtime_params_manifest_path() -> PathBuf {
    PathBuf::from(&*ELF_BASE_PATH).join(GUEST_CHECKPOINT_ARTIFACT_MANIFEST_FILE)
}

/// Defines a lazily initialized host for one guest program.
///
/// With the `sp1-builder` feature the ELF comes from the guest builder's own output directory,
/// so a locally built guest is picked up without copying it anywhere. Otherwise it is read from
/// [`ELF_BASE_PATH`], which is how deployments point at ELFs shipped alongside the binary.
macro_rules! define_host {
    ($host_fn:ident, $cell_name:ident, $builder_path:expr, $elf_file:expr) => {
        static $cell_name: OnceCell<Arc<SP1Host>> = OnceCell::const_new();

        /// Lazily initializes the host on first call and returns the shared
        /// instance. Subsequent calls return the cached host and ignore the
        /// `config` argument — callers within a single binary are expected to
        /// pass a consistent config.
        pub async fn $host_fn(config: SP1HostConfig) -> &'static Arc<SP1Host> {
            $cell_name
                .get_or_init(|| async {
                    #[cfg(feature = "sp1-builder")]
                    let elf_path = $builder_path.to_owned();
                    #[cfg(not(feature = "sp1-builder"))]
                    let elf_path = format!("{}/{}", *ELF_BASE_PATH, $elf_file);

                    let elf = fs::read(&elf_path)
                        .unwrap_or_else(|e| panic!("failed to read ELF file from {elf_path}: {e}"));
                    Arc::new(SP1Host::init_with_config(&elf, config).await)
                })
                .await
        }
    };
}

define_host!(
    checkpoint_host,
    CHECKPOINT_HOST,
    CHECKPOINT_ELF_PATH,
    "guest-checkpoint.elf"
);
