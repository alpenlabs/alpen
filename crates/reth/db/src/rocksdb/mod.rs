use rockbound::{schema::ColumnFamilyName, Schema};

mod db;
mod schema;

pub const ROCKSDB_NAME: &str = "express-reth";

pub const STORE_COLUMN_FAMILIES: &[ColumnFamilyName] = &[
    schema::BlockWitnessSchema::COLUMN_FAMILY_NAME,
    schema::BlockHashByNumber::COLUMN_FAMILY_NAME,
];

pub use db::WitnessDB;
