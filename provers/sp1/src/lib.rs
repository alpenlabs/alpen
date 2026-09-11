//! Public paths to the SP1 guest artifacts produced by this crate's build script.
//!
//! Artifacts are emitted into `<crate>/generated/` (see `build.rs`); the constants below point
//! at those stable paths rather than into cargo's `target/`. The files only exist once the crate
//! has been built with `BUILD_ELF=1`.

/// Path to the compiled checkpoint guest ELF.
pub const CHECKPOINT_ELF_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/generated/guest-checkpoint.elf"
);
