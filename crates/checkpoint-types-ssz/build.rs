use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed=specs/");

    if std::path::Path::new("specs/checkpoint_types.ssz").exists() {
        ssz_codegen::build_ssz_files(
            &["checkpoint_types.ssz"],
            "specs/",
            &[],
            out_dir.join("generated_ssz.rs").to_str().unwrap(),
            ssz_codegen::ModuleGeneration::SingleModule,
        )
        .expect("Failed to generate SSZ code");
    } else {
        std::fs::write(
            out_dir.join("generated_ssz.rs"),
            "// SSZ schemas not yet defined\n",
        )
        .expect("Failed to create placeholder");
    }
}
