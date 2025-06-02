use std::{io::Error, path::Path};

include!(concat!(env!("OUT_DIR"), "/methods.rs"));

/// Exports ELF files to the specified directory.
///
/// Creates the output directory if it doesn't exist and copies all `.elf` files
/// from guest program directories into it.
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

/// Migrates guest program ELF to the destination.
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
                migrate_elf(&file_path, destination)?;
            }
        }
    }
    Ok(())
}

/// Migrates a single ELF file to the destination directory.
fn migrate_elf(source_file: &Path, destination_dir: &Path) -> Result<(), Error> {
    if source_file.is_file()
        && source_file
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("elf"))
    {
        let file_name = source_file
            .file_name()
            .ok_or_else(|| Error::other("Invalid file name"))?;
        let destination_file = destination_dir.join(file_name);
        fs::copy(source_file, &destination_file)?;
    }
    Ok(())
}
