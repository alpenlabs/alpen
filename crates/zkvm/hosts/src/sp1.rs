use std::{
    env::var,
    sync::{Arc, LazyLock},
    time::Duration,
};

#[cfg(feature = "sp1-builder")]
use strata_sp1_guest_builder::*;
use zkaleido_sp1_host::{SP1Host, SP1HostConfig};

pub static ELF_BASE_PATH: LazyLock<String> =
    LazyLock::new(|| var("ELF_BASE_PATH").unwrap_or_else(|_| "elfs/sp1".to_string()));

macro_rules! define_host {
    ($host_name:ident, $guest_const:ident, $elf_file:expr) => {
        #[cfg(feature = "sp1-builder")]
        pub static $host_name: LazyLock<Arc<SP1Host>> =
            LazyLock::new(|| Arc::new(init_host(load_embedded_elf($guest_const), None)));

        #[cfg(not(feature = "sp1-builder"))]
        pub static $host_name: LazyLock<Arc<SP1Host>> =
            LazyLock::new(|| Arc::new(init_host(load_elf_file($elf_file), None)));
    };
}

#[cfg(feature = "sp1-builder")]
fn load_embedded_elf(embedded: &[u8]) -> Vec<u8> {
    embedded.to_vec()
}

#[cfg(not(feature = "sp1-builder"))]
fn load_elf_file(file_name: &str) -> Vec<u8> {
    let elf_path = format!("{}/{}", *ELF_BASE_PATH, file_name);
    std::fs::read(&elf_path).unwrap_or_else(|_| panic!("Failed to read ELF file from {elf_path}"))
}

fn init_host(elf: Vec<u8>, deadline: Option<Duration>) -> SP1Host {
    let mut config = SP1HostConfig::from_env();
    if let Some(deadline) = deadline {
        config = config.with_deadline(deadline);
    }

    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("failed to build SP1 host init runtime")
                .block_on(SP1Host::init_with_config(&elf, config))
        })
        .join()
        .expect("SP1 host init thread panicked")
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build SP1 host init runtime")
            .block_on(SP1Host::init_with_config(&elf, config))
    }
}

/// Returns a checkpoint SP1 host with a per-call proof deadline.
#[cfg(feature = "sp1-builder")]
pub fn checkpoint_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_embedded_elf(GUEST_CHECKPOINT_ELF), Some(deadline))
}

/// Returns a checkpoint SP1 host with a per-call proof deadline.
#[cfg(not(feature = "sp1-builder"))]
pub fn checkpoint_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_elf_file("guest-checkpoint.elf"), Some(deadline))
}

/// Returns an Alpen chunk SP1 host with a per-call proof deadline.
#[cfg(feature = "sp1-builder")]
pub fn alpen_chunk_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_embedded_elf(GUEST_ALPEN_CHUNK_ELF), Some(deadline))
}

/// Returns an Alpen chunk SP1 host with a per-call proof deadline.
#[cfg(not(feature = "sp1-builder"))]
pub fn alpen_chunk_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_elf_file("guest-alpen-chunk.elf"), Some(deadline))
}

/// Returns an Alpen account SP1 host with a per-call proof deadline.
#[cfg(feature = "sp1-builder")]
pub fn alpen_acct_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_embedded_elf(GUEST_ALPEN_ACCT_ELF), Some(deadline))
}

/// Returns an Alpen account SP1 host with a per-call proof deadline.
#[cfg(not(feature = "sp1-builder"))]
pub fn alpen_acct_host_with_deadline(deadline: Duration) -> SP1Host {
    init_host(load_elf_file("guest-alpen-acct.elf"), Some(deadline))
}

// Define hosts using the macro.
define_host!(
    CHECKPOINT_HOST,
    GUEST_CHECKPOINT_ELF,
    "guest-checkpoint.elf"
);
define_host!(
    ALPEN_CHUNK_HOST,
    GUEST_ALPEN_CHUNK_ELF,
    "guest-alpen-chunk.elf"
);
define_host!(
    ALPEN_ACCT_HOST,
    GUEST_ALPEN_ACCT_ELF,
    "guest-alpen-acct.elf"
);
