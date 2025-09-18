fn main() {
    std::env::set_var("CC", "clang");
    std::env::set_var(
        "CFLAGS_riscv32im_risc0_zkvm_elf",
        "-target riscv32-unknown-elf",
    );
    if std::env::var("SKIP_GUEST_BUILD").is_err()
        && std::env::var("CARGO_CFG_CLIPPY").is_err()
        && cfg!(not(debug_assertions))
    {
        risc0_build::embed_methods();
    } else {
        // Return mock ELF
        let out_dir = std::env::var_os("OUT_DIR").unwrap();
        let out_dir = std::path::Path::new(&out_dir);
        let methods_path = out_dir.join("methods.rs");

        let elf = r#"
            pub const GUEST_RISC0_EVM_EE_STF_ELF: &[u8] = &[];
            pub const GUEST_RISC0_EVM_EE_STF_ID: &[u32; 8] = &[0u32; 8];

            pub const GUEST_RISC0_CL_STF_ELF: &[u8] = &[];
            pub const GUEST_RISC0_CL_STF_ID: [u32; 8] = [0u32; 8];

            pub const GUEST_RISC0_CL_AGG_ELF: &[u8] = &[];
            pub const GUEST_RISC0_CL_AGG_ID: [u32; 8] = [0u32; 8];

            pub const GUEST_RISC0_CHECKPOINT_ELF: &[u8] = &[];
            pub const GUEST_RISC0_CHECKPOINT_ID: [u32; 8] = [0u32; 8];
        "#;

        std::fs::write(methods_path, elf).expect("Failed to write mock rollup elf");
    }
}
