use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(feature = "sp1-dev", not(debug_assertions)))] {
        use bincode::{deserialize, serialize};
        use cargo_metadata::MetadataCommand;
        use sha2::{Digest, Sha256};
        use sp1_helper::{build_program_with_args, BuildArgs};
        use sp1_sdk::{HashableKey, ProverClient, SP1VerifyingKey};
    }
}

// Guest program names
const EVM_EE_STF: &str = "guest-evm-ee-stf";
const CL_STF: &str = "guest-cl-stf";
const BTC_BLOCKSPACE: &str = "guest-btc-blockspace";
const CHECKPOINT: &str = "guest-checkpoint";

/// Returns a map of program dependencies.
fn get_program_dependencies() -> HashMap<&'static str, Vec<&'static str>> {
    let mut dependencies = HashMap::new();
    dependencies.insert(CL_STF, vec![BTC_BLOCKSPACE, EVM_EE_STF]);
    dependencies.insert(CHECKPOINT, vec![CL_STF]);
    dependencies
}

fn main() {
    // List of guest programs to build
    let guest_programs = [BTC_BLOCKSPACE, EVM_EE_STF, CL_STF, CHECKPOINT];

    // HashSet to keep track of programs that have been built
    let mut built_programs = HashSet::new();

    // Get the dependencies between programs
    let dependencies = get_program_dependencies();

    // HashMap to store results: mapping from elf_name to (elf_contents, vk_hash_u32, vk_hash_str)
    let mut results = HashMap::new();

    // HashMap to store vk hashes of programs
    let mut vk_hashes = HashMap::new();

    // Build each guest program along with its dependencies
    for program in &guest_programs {
        build_program_with_dependencies(
            program,
            &dependencies,
            &mut built_programs,
            &mut results,
            &mut vk_hashes,
        );
    }

    // String to accumulate the contents of methods.rs file
    // Start with the necessary use statements
    let mut methods_file_content = String::from(
        r#"
use once_cell::sync::Lazy;
use std::fs;
"#,
    );

    // Write the methods.rs file with ELF contents and VK hashes
    for (program_name, (vk_hash_u32, vk_hash_str)) in &results {
        let program_name_upper = program_name.to_uppercase().replace("-", "_");
        let base_path = Path::new(program_name)
            .canonicalize()
            .expect("Cache directory not found");
        let base_path_str = base_path
            .to_str()
            .expect("Failed to convert path to string");

        let full_path_str = format!("{base_path_str}/cache/{program_name}");
        methods_file_content.push_str(&format!(
            r#"
pub static {program_name_upper}_ELF: Lazy<Vec<u8>> = Lazy::new(||{{ fs::read("{full_path_str}.elf").expect("Cannot find ELF") }});
pub static {program_name_upper}_PK: Lazy<Vec<u8>> = Lazy::new(||{{ fs::read("{full_path_str}.pk").expect("Cannot find PK") }});
pub static {program_name_upper}_VK: Lazy<Vec<u8>> = Lazy::new(||{{ fs::read("{full_path_str}.vk").expect("Cannot find VK") }});
pub const {program_name_upper}_VK_HASH_U32: &[u32] = &{vk_hash_u32:?};
pub const {program_name_upper}_VK_HASH_STR: &str = "{vk_hash_str}";
"#
        ));
    }

    // Write the accumulated methods_file_content to methods.rs in the output directory
    let out_dir = get_output_dir();
    let methods_path = out_dir.join("methods.rs");
    fs::write(&methods_path, methods_file_content).unwrap_or_else(|e| {
        panic!(
            "Failed to write methods.rs file at {}: {}",
            methods_path.display(),
            e
        )
    });
}

/// Recursively builds the given program along with its dependencies.
fn build_program_with_dependencies(
    program: &str,
    dependencies: &HashMap<&str, Vec<&str>>,
    built_programs: &mut HashSet<String>,
    results: &mut HashMap<String, ([u32; 8], String)>,
    vk_hashes: &mut HashMap<String, [u32; 8]>,
) {
    // If the program has already been built, return early
    if built_programs.contains(program) {
        return;
    }

    // Build dependencies first
    if let Some(deps) = dependencies.get(program) {
        for dep in deps {
            build_program_with_dependencies(dep, dependencies, built_programs, results, vk_hashes);
        }

        // After dependencies are built, write vks.rs for the current program
        let mut vks_content = String::new();
        for dep in deps {
            if let Some(vk_hash) = vk_hashes.get(*dep) {
                let elf_name = format!("{}_ELF", dep.to_uppercase().replace("-", "_"));
                let elf_name_id = format!("{elf_name}_ID");
                vks_content.push_str(&format!(
                    "pub const {elf_name_id}: &[u32; 8] = &{vk_hash:?};\n"
                ));
            }
        }

        // Only write vks.rs if there are dependencies
        if !vks_content.is_empty() {
            let vks_path = Path::new(program).join("src").join("vks.rs");
            fs::write(&vks_path, vks_content)
                .unwrap_or_else(|e| panic!("Failed to write vks.rs for {program}: {e}"));
        }
    }

    // Build the program and generate ELF contents and VK hash
    let (vk_hash_u32, vk_hash_str) = generate_elf_contents_and_vk_hash(program);

    // Store the results
    results.insert(program.to_string(), (vk_hash_u32, vk_hash_str));
    vk_hashes.insert(program.to_string(), vk_hash_u32);
    built_programs.insert(program.to_string());
}

/// Returns the output directory for the build artifacts.
fn get_output_dir() -> PathBuf {
    env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .expect("OUT_DIR environment variable is not set. Cannot determine output directory.")
}

/// Checks if the cache is valid by comparing the expected ID with the saved ID.
#[cfg(all(feature = "sp1-dev", not(debug_assertions)))]
fn is_cache_valid(expected_id: &[u8; 32], paths: &[PathBuf; 4]) -> bool {
    // Check if any required files are missing
    if paths.iter().any(|path| !path.exists()) {
        return false;
    }

    // Attempt to read the saved ID
    let saved_id = match fs::read(&paths[1]) {
        Ok(data) => data,
        Err(_) => return false,
    };

    expected_id == saved_id.as_slice()
}

/// Ensures the cache is valid and returns the ELF contents and SP1 Verifying Key.
#[cfg(all(feature = "sp1-dev", not(debug_assertions)))]
fn ensure_cache_validity(program: &str) -> Result<SP1VerifyingKey, String> {
    let cache_dir = format!("{}/cache", program);
    let paths = ["elf", "id", "vk", "pk"]
        .map(|file| Path::new(&cache_dir).join(format!("{}.{}", program, file)));

    // Attempt to read the ELF file
    let elf = fs::read(&paths[0])
        .map_err(|e| format!("Failed to read ELF file {}: {}", paths[0].display(), e))?;
    let elf_hash: [u8; 32] = Sha256::digest(&elf).into();

    if !is_cache_valid(&elf_hash, &paths) {
        // Cache is invalid, need to generate vk and pk
        let client = ProverClient::from_env();
        let (pk, vk) = client.setup(&elf);

        fs::write(&paths[1], elf_hash)
            .map_err(|e| format!("Failed to write ID file {}: {}", paths[1].display(), e))?;

        fs::write(&paths[2], serialize(&vk).expect("VK serialization failed"))
            .map_err(|e| format!("Failed to write VK file {}: {}", paths[2].display(), e))?;

        fs::write(&paths[3], serialize(&pk).expect("PK serialization failed"))
            .map_err(|e| format!("Failed to write PK file {}: {}", paths[3].display(), e))?;

        Ok(vk)
    } else {
        // Cache is valid, read the VK
        let serialized_vk = fs::read(&paths[2])
            .map_err(|e| format!("Failed to read VK file {}: {}", paths[2].display(), e))?;
        let vk: SP1VerifyingKey =
            deserialize(&serialized_vk).map_err(|e| format!("VK deserialization failed: {}", e))?;
        Ok(vk)
    }
}

/// Generates the ELF contents and VK hash for a given program.
#[cfg(all(feature = "sp1-dev", not(debug_assertions)))]
fn generate_elf_contents_and_vk_hash(program: &str) -> ([u32; 8], String) {
    // Check if the Clippy linter is enabled by examining the "RUSTC_WORKSPACE_WRAPPER" environment
    // variable. If it contains "clippy-driver", Clippy is active; in that case, return mock ELF
    // contents and VK hash.
    let is_clippy_enabled = std::env::var("RUSTC_WORKSPACE_WRAPPER")
        .map(|val| val.contains("clippy-driver"))
        .unwrap_or(false);

    if is_clippy_enabled {
        return get_mock_elf_contents_and_vk_hash();
    }

    let mut build_args = BuildArgs {
        ..Default::default()
    };

    // Tell Cargo to re-run the build script if ZKVM_MOCK changes.
    println!("cargo:rerun-if-env-changed=ZKVM_MOCK");

    // If the environment variable "ZKVM_MOCK" is set to "1" or "true" (case-insensitive),
    // then do not activate SP1 proof recursive verification. Instead make the recursive
    // verification a no-op
    build_args.features = if std::env::var("ZKVM_MOCK")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
    {
        println!("cargo:warning=ZKVM_MOCK is set. ----------------------------------------");
        println!("cargo:warning=ZKVM_MOCK is set. This should never be used in production.");
        println!("cargo:warning=ZKVM_MOCK is set. ----------------------------------------");
        vec!["mock-verify".to_string()]
    } else {
        println!("cargo:warning=ZKVM_MOCK is not set. Good for production");
        vec!["zkvm-verify".to_string()]
    };

    // In the Docker build, override the guest program's Cargo workspace root with the Strata
    // workspace root so Docker mounts the entire Strata workspace, enabling the guest program
    // to import Strata crates relatively.
    #[cfg(feature = "docker-build")]
    {
        build_args.docker = true;
        build_args.workspace_directory = Some("../../".to_owned());
    }

    // Build the program with the specified arguments
    // Note: SP1_v4's build_programs_with_args does not handle ELF migration
    // Applying a temporary workaround; remove once SP1 supports ELF migration internally
    build_program_with_args(program, build_args);
    migrate_elf(program);

    // Now, ensure cache validity
    let vk = ensure_cache_validity(program)
        .expect("Failed to ensure cache validity after building program");
    (vk.hash_u32(), vk.bytes32())
}

#[cfg(debug_assertions)]
fn generate_elf_contents_and_vk_hash(_program: &str) -> ([u32; 8], String) {
    get_mock_elf_contents_and_vk_hash()
}

fn get_mock_elf_contents_and_vk_hash() -> ([u32; 8], String) {
    (
        [0u32; 8],
        "0x0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
    )
}

/// Copies the compiled ELF file of the specified program to its cache directory.
#[cfg(all(feature = "sp1-dev", not(debug_assertions)))]
fn migrate_elf(program: &str) {
    // Get the build directory from the environment
    let sp1_build_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"));

    // Form the path to the program directory
    let program_path = sp1_build_dir.join(program);

    // Fetch metadata for this program
    let metadata = MetadataCommand::new()
        .manifest_path(program_path.join("Cargo.toml"))
        .exec()
        .expect("Failed to get metadata");

    // Use the root package name as the built ELF name
    let built_elf_name = metadata
        .root_package()
        .expect("Failed to get root package")
        .name
        .clone();

    // Create the cache directory
    let cache_dir = program_path.join("cache");
    fs::create_dir_all(&cache_dir).expect("failed to create cache dir");

    // Destination path: cache/program.elf
    let destination_elf_path = cache_dir.join(format!("{}.elf", program));

    // Source path: program/target/elf-compilation/.../release/{built_elf_name}
    let elf_subdir = if cfg!(feature = "docker-build") {
        "docker/riscv32im-succinct-zkvm-elf"
    } else {
        "riscv32im-succinct-zkvm-elf"
    };

    let built_elf_path = program_path
        .join("target")
        .join("elf-compilation")
        .join(elf_subdir)
        .join("release")
        .join(&built_elf_name);

    eprintln!("Got the source: {:?}", built_elf_path);
    eprintln!("Got the destination: {:?}", destination_elf_path);

    // Copy the file
    fs::copy(&built_elf_path, &destination_elf_path)
        .expect("Failed to copy the built ELF file to the cache directory");
}
