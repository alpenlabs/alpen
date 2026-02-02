use std::{env::var, fs, path::Path};

use ssz_codegen::{ModuleGeneration, build_ssz_files};

fn main() {
    let out_dir = var("OUT_DIR").expect("OUT_DIR not set by cargo");
    let output_path = Path::new(&out_dir).join("generated.rs");

    let entry_points = ["log.ssz", "manifest.ssz"];
    let base_dir = "ssz";
    let crates = ["strata_identifiers"];

    build_ssz_files(
        &entry_points,
        base_dir,
        &crates,
        output_path.to_str().expect("output path is valid"),
        ModuleGeneration::NestedModules,
    )
    .expect("Failed to generate SSZ types");

    // TODO: this is annoying. Will replace with proper support for rkyv in ssz-gen
    let mut generated = fs::read_to_string(&output_path).expect("read generated SSZ code");
    if !generated.contains("use rkyv::{Archive as RkyvArchive") {
        generated = generated.replace(
            "    use ssz::view::*;\n",
            "    use ssz::view::*;\n    use rkyv::{Archive as RkyvArchive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};\n",
        );
        fs::write(&output_path, generated).expect("write generated SSZ code");
    }

    println!("cargo:rerun-if-changed=ssz/log.ssz");
    println!("cargo:rerun-if-changed=ssz/manifest.ssz");
}
