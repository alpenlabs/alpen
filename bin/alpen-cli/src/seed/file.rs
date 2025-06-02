use std::{
    fs::{remove_file, File},
    io::{ErrorKind, Read, Write},
    path::PathBuf,
};

use terrors::OneOf;

use super::{EncryptedSeed, EncryptedSeedPersister, PersisterErr};

#[derive(Clone, Debug)]
pub struct FilePersister {
    file: PathBuf,
}

impl FilePersister {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }
}

impl EncryptedSeedPersister for FilePersister {
    fn save(&self, seed: &EncryptedSeed) -> Result<(), PersisterErr> {
        let mut file = File::options()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.file)?;
        file.write_all(&seed.0)?;
        file.sync_all()?;
        Ok(())
    }

    fn load(&self) -> Result<Option<EncryptedSeed>, PersisterErr> {
        let mut file = match File::options().read(true).open(&self.file) {
            Ok(f) => f,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(OneOf::new(e)),
        };
        let mut buf = [0u8; EncryptedSeed::LEN];
        let bytes_read = file.read(&mut buf)?;
        if bytes_read != buf.len() {
            self.delete()?;
            return Ok(None);
        }
        Ok(Some(EncryptedSeed(buf)))
    }

    fn delete(&self) -> Result<(), PersisterErr> {
        remove_file(&self.file).map_err(OneOf::new)
    }
}
