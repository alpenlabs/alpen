//! Build script for the SP1 guest programs used by Alpen proof workflows.
//!
//! Compiled ELFs are emitted to `<crate>/generated/<guest>.elf` regardless of the `docker-build`
//! feature, so consumers can reference a stable path that survives `cargo clean`. Alongside each
//! ELF the guest's verifying key is derived and written as three files: `<guest>.vk-hash` (the
//! `bytes32` program ID), `<guest>.predicate` (`Sp1Groth16:<hex>`, the form `strata-datatool`
//! reads when building params), and `<guest>.artifact-manifest.json`, which binds the program ID
//! to the runtime params baked into the ELF so a node can refuse to run a mismatched pair.
//!
//! # Environment
//!
//! Both steps are off by default and opt-in, because both are slow and most builds of this
//! workspace only need the crate to compile. The files in `<crate>/generated/` survive `cargo
//! clean`, so a build that skips these steps still leaves whatever was built earlier in place.
//!
//! - **`BUILD_ELF`** — set to `1`/`true` to compile the guest programs. Ignored under `cargo
//!   clippy`, which only needs the crate to typecheck.
//! - **`BUILD_VKEY`** — set to `1`/`true` to derive each guest's vk and write the `*.vk-hash`,
//!   `*.predicate` and `*.artifact-manifest.json` files. Requires the ELFs to exist, so it implies
//!   `BUILD_ELF`.
//! - **`CHECKPOINT_RUNTIME_PARAMS_PATH`** — path to the OL params (or bare OL runtime params) JSON
//!   to bake into the checkpoint guest. Required whenever the guest is actually built, because the
//!   params are part of what the ELF proves.
//! - **`ZKVM_MOCK`** — set to `1`/`true` to build guests with recursive proof verification stubbed
//!   out. Only for local runs and perf evaluation, never for anything published.
//!
//! # Features
//!
//! - **`docker-build`** — when enabled, guest programs are compiled inside Docker via
//!   `build_program_with_args` instead of locally. The output location is unchanged.

use std::{env, fs, path::Path};

use sp1_build::{build_program_with_args, BuildArgs};
use sp1_sdk::{
    blocking::{Prover, ProverClient},
    HashableKey, ProvingKey, SP1VerifyingKey,
};
use sp1_verifier::{GROTH16_VK_BYTES, VK_ROOT_BYTES};
use ssz::Encode;
use strata_ol_params::{OLParams, OLRuntimeParams};
use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

const GENERATED_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/generated");

/// Guest crate directory. The directory name doubles as the artifact base name, so this yields
/// `guest-checkpoint.elf` and friends.
const CHECKPOINT: &str = "guest-checkpoint";

const RUNTIME_PARAMS_PATH_VAR: &str = "CHECKPOINT_RUNTIME_PARAMS_PATH";

fn main() {
    println!("cargo:rerun-if-env-changed=BUILD_ELF");
    println!("cargo:rerun-if-env-changed=BUILD_VKEY");
    println!("cargo:rerun-if-env-changed=ZKVM_MOCK");
    println!("cargo:rerun-if-env-changed={RUNTIME_PARAMS_PATH_VAR}");

    // clippy only needs the crate to typecheck, so it never builds guests whatever is set.
    if is_clippy() {
        return;
    }

    // Deriving a vk reads the ELF back off disk, so asking for the vk implies building the ELF.
    let build_vkey = is_enabled("BUILD_VKEY");
    if !is_enabled("BUILD_ELF") && !build_vkey {
        println!("cargo:warning=BUILD_ELF/BUILD_VKEY unset; skipping SP1 guest build");
        return;
    }

    println!("cargo:warning=exporting SP1 guest artifacts to {GENERATED_DIR}");

    // Must precede the build: the guest includes the generated params as a source file.
    let runtime_params_hash = write_runtime_params();

    build_guest(CHECKPOINT);
    if build_vkey {
        emit_vkey_artifacts(CHECKPOINT, runtime_params_hash);
    }
}

fn build_guest(guest: &str) {
    let build_args = BuildArgs {
        output_directory: Some(GENERATED_DIR.to_owned()),
        elf_name: Some(format!("{guest}.elf")),
        features: vec![verification_feature().to_owned()],
        // In the Docker build, override the guest's own Cargo workspace root with the Alpen
        // workspace root so Docker mounts the whole workspace and the guest can import Alpen
        // crates by relative path.
        #[cfg(feature = "docker-build")]
        docker: true,
        #[cfg(feature = "docker-build")]
        workspace_directory: Some("../../".to_owned()),
        ..BuildArgs::default()
    };
    build_program_with_args(guest, build_args);
}

/// Picks the guest feature that decides whether recursive SP1 proof verification runs for real.
fn verification_feature() -> &'static str {
    if is_enabled("ZKVM_MOCK") {
        println!(
            "cargo:warning=ZKVM_MOCK is set: guest proof verification is a no-op, so the resulting ELF must never be used in production"
        );
        "mock-verify"
    } else {
        "zkvm-verify"
    }
}

/// Bakes the OL runtime params into the checkpoint guest by generating the source file the guest
/// includes, and returns their hash for the artifact manifest.
///
/// The file accepts either a bare [`OLRuntimeParams`] document or a full [`OLParams`] one, so the
/// same `ol-params.json` a node loads can be pointed at here without being trimmed first.
fn write_runtime_params() -> [u8; 32] {
    let path = env::var_os(RUNTIME_PARAMS_PATH_VAR).unwrap_or_else(|| {
        panic!("{RUNTIME_PARAMS_PATH_VAR} must be set to build the checkpoint guest ELF")
    });
    let path = Path::new(&path);
    println!("cargo:rerun-if-changed={}", path.display());

    let json = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "read {RUNTIME_PARAMS_PATH_VAR} from {}: {e}",
            path.display()
        )
    });
    let runtime_params = serde_json::from_str::<OLRuntimeParams>(&json)
        .or_else(|_| serde_json::from_str::<OLParams>(&json).map(|p| p.runtime_params()))
        .unwrap_or_else(|e| {
            panic!(
                "parse {RUNTIME_PARAMS_PATH_VAR} at {} as OL runtime params or full OL params: {e}",
                path.display()
            )
        });

    let ssz = runtime_params.as_ssz_bytes();
    let out_path = Path::new(CHECKPOINT).join("src").join("runtime_params.rs");
    fs::write(
        &out_path,
        format!(
            "// Generated by `build.rs`; do not edit.\n\
             pub const CHECKPOINT_RUNTIME_PARAMS_SSZ: &[u8] = &{ssz:?};\n"
        ),
    )
    .unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));

    runtime_params.hash()
}

/// Derives the guest's verifying key from the freshly built ELF and writes the metadata files
/// that params generation, artifact publishing and node startup read.
fn emit_vkey_artifacts(guest: &str, runtime_params_hash: [u8; 32]) {
    let elf_path = Path::new(GENERATED_DIR).join(format!("{guest}.elf"));
    let elf = fs::read(&elf_path)
        .unwrap_or_else(|e| panic!("read built ELF {}: {e}", elf_path.display()));

    let prover = ProverClient::builder().cpu().build();
    let pk = prover
        .setup(elf.into())
        .unwrap_or_else(|e| panic!("sp1 key setup for {guest}: {e}"));
    let vk = pk.verifying_key();
    let program_id = program_id(vk);

    write_artifact(guest, "vk-hash", &vk.bytes32());
    write_artifact(guest, "predicate", &predicate_string(&program_id));
    write_artifact(
        guest,
        "artifact-manifest.json",
        &artifact_manifest(&program_id, &runtime_params_hash),
    );
}

/// Renders the manifest binding a built ELF's program ID to the runtime params baked into it.
///
/// A node checks both halves at startup: the program ID against the ELF it loaded, and the params
/// hash against the ones it was configured with.
fn artifact_manifest(program_id: &[u8; 32], runtime_params_hash: &[u8; 32]) -> String {
    let manifest = serde_json::json!({
        "schema": 1,
        "program_id": hex::encode(program_id),
        "runtime_params_hash": hex::encode(runtime_params_hash),
    });
    serde_json::to_string_pretty(&manifest).expect("manifest must serialize")
}

/// Renders the predicate key string for a guest's BN254 program ID.
///
/// The hex payload is the canonical uncompressed encoding of an [`SP1Groth16Verifier`], which is
/// what the runtime `Sp1Groth16` predicate verifier in `strata-predicate` decodes. The verifier
/// object embeds the SP1 circuit VK merged with the program-specific ID and the VK root.
fn predicate_string(program_id: &[u8; 32]) -> String {
    let verifier = SP1Groth16Verifier::load(&GROTH16_VK_BYTES, *program_id, *VK_ROOT_BYTES, true)
        .unwrap_or_else(|e| panic!("load SP1 Groth16 verifier: {e}"));
    format!(
        "Sp1Groth16:{}",
        hex::encode(verifier.to_uncompressed_bytes())
    )
}

/// Computes the BN254 program ID for a verifying key.
///
/// Equivalent in value to `HashableKey::bytes32_raw`, but without its panic: `bytes32_raw` does
/// `result[1..].copy_from_slice(&digest.to_bytes_be())`, which assumes the big-endian digest is
/// exactly 31 bytes — yet `to_bytes_be` strips leading zero bytes, so a digest with extra leading
/// zeros serializes shorter (e.g. 30 bytes) and the copy panics. `bytes32()` is the same value
/// zero-padded to a fixed 32 bytes, so we decode that instead and stay robust to whatever digest a
/// given guest ELF happens to produce.
///
/// N.B. The upstream fix: <https://github.com/succinctlabs/sp1/pull/2508>
fn program_id(vk: &SP1VerifyingKey) -> [u8; 32] {
    let bytes32 = vk.bytes32();
    let hex = bytes32.strip_prefix("0x").unwrap_or(&bytes32);
    assert_eq!(hex.len(), 64, "bytes32() must encode exactly 32 bytes");
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte =
            u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("bytes32() returns valid hex");
    }
    out
}

fn write_artifact(guest: &str, suffix: &str, contents: &str) {
    let path = Path::new(GENERATED_DIR).join(format!("{guest}.{suffix}"));
    fs::write(&path, format!("{contents}\n"))
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("cargo:warning=wrote {}", path.display());
}

fn is_clippy() -> bool {
    env::var("RUSTC_WORKSPACE_WRAPPER")
        .map(|v| v.contains("clippy-driver"))
        .unwrap_or(false)
}

/// Reads an opt-in flag: set and equal to `1` or `true` (any case) enables it.
fn is_enabled(var: &str) -> bool {
    env::var(var)
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}
