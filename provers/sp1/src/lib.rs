use std::{io::Error, path::Path};

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

/// Exports guest artifact files to the specified directory.
///
/// Creates the output directory if it doesn't exist and copies all `.elf` files
/// plus their runtime params hash sidecars from guest program directories into
/// it.
///
/// # Arguments
///
/// * `elf_path` - The path to the directory where ELF files will be exported.
///
/// # Errors
///
/// Returns an error if directory creation or file operations fail.
pub fn export_elf<P: AsRef<Path>>(elf_path: P) -> Result<(), Error> {
    let elf_path = elf_path.as_ref();
    fs::create_dir_all(elf_path)?;

    let builder_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    for entry in fs::read_dir(builder_dir)? {
        let path = entry?.path();
        migrate_guest_program(&path, elf_path)?;
    }

    Ok(())
}

/// Migrates guest program artifacts to the destination.
fn migrate_guest_program(source: &Path, destination: &Path) -> Result<(), Error> {
    if source.is_dir()
        && source
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.starts_with("guest-"))
    {
        let cache_dir = source.join("cache");
        if cache_dir.is_dir() {
            for file in fs::read_dir(&cache_dir)? {
                let file_path = file?.path();
                copy_artifact(&file_path, destination)?;
            }
        }
    }
    Ok(())
}

/// Copies a guest artifact file to the destination directory.
fn copy_artifact(source_file: &Path, destination_dir: &Path) -> Result<(), Error> {
    if source_file.is_file() && is_artifact(source_file) {
        let file_name = source_file
            .file_name()
            .ok_or_else(|| Error::other("Invalid file name"))?;
        let destination_file = destination_dir.join(file_name);
        fs::copy(source_file, &destination_file)?;
    }
    Ok(())
}

fn is_artifact(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("elf"))
        || path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".runtime-params-hash"))
}
