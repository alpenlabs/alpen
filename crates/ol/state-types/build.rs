use std::{env::var, path::Path};

use ssz_codegen::{ModuleGeneration, build_ssz_files};

fn main() {
    let out_dir = var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let output_path = Path::new(&out_dir).join("generated.rs");

    let entry_points = ["state.ssz"];
    let base_dir = "ssz";
    let crates = [
        "strata_acct_types",
        "strata_identifiers",
        "strata_asm_manifest_types",
        "strata_snark_acct_types",
        "strata_predicate",
    ];

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
}
