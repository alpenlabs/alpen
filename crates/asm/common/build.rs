use std::{env::var, path::Path};

use ssz_codegen::{ModuleGeneration, build_ssz_files};

fn main() {
    let out_dir = var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let output_path = Path::new(&out_dir).join("generated.rs");

    let entry_points = ["aux.ssz", "pow.ssz", "state.ssz"];
    let base_dir = "ssz";
    let crates = ["strata_acct_types", "strata_identifiers"];

    build_ssz_files(
        &entry_points,
        base_dir,
        &crates,
        output_path.to_str().expect("output path is valid UTF-8"),
        ModuleGeneration::NestedModules,
    )
    .expect("Failed to generate SSZ types");

    println!("cargo:rerun-if-changed=ssz/aux.ssz");
    println!("cargo:rerun-if-changed=ssz/pow.ssz");
    println!("cargo:rerun-if-changed=ssz/state.ssz");
}
