mod db;
mod schema;

pub const ROCKSDB_NAME: &str = "express-reth";

pub use db::WitnessDB;
