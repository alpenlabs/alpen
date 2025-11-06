use std::path::Path;

use ssz_codegen::{ModuleGeneration, build_ssz_files};

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let output_path = Path::new(&out_dir).join("generated.rs");

    // Only files that import external crates need to be entry points.
    // Internal files like proofs.ssz are automatically resolved when imported.
    // IMPORTANT: Order matters! Files that are imported by others should be listed
    // BEFORE the files that import them to avoid duplicate errors.
    // Pattern: Process imported modules first, then files that import them.
    let entry_points = [
        // Files that are imported by others (no dependencies on other entry points)
        "outputs.ssz",      // imports strata_acct_types (external), imported by update.ssz
        "state.ssz",        // imports strata_acct_types (external), imported by update.ssz
        "accumulators.ssz", // imports proofs (internal), imported by update.ssz
        "messages.ssz",     // imports strata_acct_types (external) and proofs (internal), imported by update.ssz
        // Files that import other entry points (depend on files above)
        "update.ssz",       // imports strata_acct_types (external) and all the above modules
    ];
    let base_dir = "ssz";
    let crates = ["strata_acct_types"];

    build_ssz_files(
        &entry_points,
        base_dir,
        &crates,
        output_path.to_str().expect("output path is valid"),
        ModuleGeneration::NestedModules,
    )
    .expect("Failed to generate SSZ types");

    println!("cargo:rerun-if-changed=ssz/accumulators.ssz");
    println!("cargo:rerun-if-changed=ssz/messages.ssz");
    println!("cargo:rerun-if-changed=ssz/outputs.ssz");
    println!("cargo:rerun-if-changed=ssz/proofs.ssz");
    println!("cargo:rerun-if-changed=ssz/state.ssz");
    println!("cargo:rerun-if-changed=ssz/update.ssz");
}


