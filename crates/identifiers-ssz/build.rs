use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=specs/");

    // Generate Rust code from SSZ schemas
    // Note: This will fail until we add the actual schema files in the next commit
    // For now, we just set up the infrastructure
    if std::path::Path::new("specs/identifiers.ssz").exists() {
        ssz_codegen::build_ssz_files(
            &["identifiers.ssz"],
            "specs/",
            &[], // No external SSZ crates to import
            out_dir.join("generated_ssz.rs").to_str().unwrap(),
            ssz_codegen::ModuleGeneration::SingleModule,
        )
        .expect("Failed to generate SSZ code");
    } else {
        // Skeleton: create empty generated file so crate compiles
        std::fs::write(
            out_dir.join("generated_ssz.rs"),
            "// SSZ schemas not yet defined\n",
        )
        .expect("Failed to create placeholder");
    }
}
