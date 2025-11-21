use std::path::Path;

use ssz_codegen::{ModuleGeneration, build_ssz_files};

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let output_path = Path::new(&out_dir).join("generated.rs");

    let entry_points = [
        "outputs.ssz",
        "state.ssz",
        "accumulators.ssz",
        "messages.ssz",
        "update.ssz",
    ];
    let base_dir = "ssz";
    let crates = ["strata_acct_types"];

    build_ssz_files(
        &entry_points,
        base_dir,
        &crates,
        output_path
            .to_str()
            .expect("OUT_DIR path must be valid UTF-8"),
        ModuleGeneration::NestedModules,
    )
    .expect("failed to generate SSZ types");

    println!("cargo:rerun-if-changed=ssz/accumulators.ssz");
    println!("cargo:rerun-if-changed=ssz/messages.ssz");
    println!("cargo:rerun-if-changed=ssz/outputs.ssz");
    println!("cargo:rerun-if-changed=ssz/state.ssz");
    println!("cargo:rerun-if-changed=ssz/update.ssz");
}
