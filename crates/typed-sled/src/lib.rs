//! # typed-sled
//!
//! A type-safe wrapper around the sled embedded database.
//!
//! This library provides a schema-based approach to working with sled,
//! ensuring compile-time type safety for keys and values while leveraging
//! efficient binary serialization.
//!
//! ## Features
//!
//! - **Type Safety**: Schema-based table definitions with associated key/value types
//! - **Serialization**: Borsh-based efficient binary encoding
//! - **Transactions**: Multi-table atomic operations
//! - **Error Handling**: Comprehensive error types with proper error chaining
//!
//! ## Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use borsh::{BorshDeserialize, BorshSerialize};
//! use typed_sled::{DbResult, Schema, SledDb};
//!
//! #[derive(BorshSerialize, BorshDeserialize, Debug)]
//! struct User {
//!     id: u32,
//!     name: String,
//! }
//!
//! #[derive(Debug)]
//! struct UserSchema;
//!
//! impl Schema for UserSchema {
//!     const TREE_NAME: &'static str = "users";
//!     type Key = u32;
//!     type Value = User;
//! }
//!
//! fn main() -> DbResult<()> {
//!     let sled_db = Arc::new(sled::open("mydb").unwrap());
//!     let db = SledDb::new(sled_db, &["users"])?;
//!
//!     let user = User {
//!         id: 1,
//!         name: "Alice".to_string(),
//!     };
//!     db.put::<UserSchema>(&1, &user)?;
//!
//!     let retrieved = db.get::<UserSchema>(&1)?;
//!     println!("{:?}", retrieved);
//!
//!     Ok(())
//! }
//! ```

pub mod codec;
// NOTE: This contains borsh specific derivation and some tweaks to have integers be big endian
// encoded. Might need to feature gate for borsh.
pub mod codec_derive;
pub mod db;
pub mod error;
pub mod schema;
pub mod tree;

// Re-export main types
pub use codec::{CodecError, CodecResult, KeyCodec, ValueCodec};
pub use db::SledDb;
pub use schema::{Schema, TreeName};
pub use tree::SledTree;
