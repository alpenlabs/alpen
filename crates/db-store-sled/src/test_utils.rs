use std::sync::Arc;

use typed_sled::SledDb;

use crate::{SledBackend, SledDbConfig};

pub fn get_test_sled_db() -> SledDb {
    let db = sled::Config::new().temporary(true).open().unwrap();
    SledDb::new(db).unwrap()
}

pub fn get_test_sled_config() -> SledDbConfig {
    SledDbConfig::test()
}

pub fn get_test_sled_backend() -> Arc<SledBackend> {
    let sdb = Arc::new(get_test_sled_db());
    let cnf = get_test_sled_config();
    SledBackend::new(sdb, cnf).map(Arc::new).unwrap()
}
