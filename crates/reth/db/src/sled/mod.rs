mod db;
mod schema;

pub const SLED_NAME: &str = "express-reth";

pub use db::WitnessDB;