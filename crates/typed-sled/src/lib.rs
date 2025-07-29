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
//! use typed_sled::{CodecError, KeyCodec, Schema, SledDb, TreeName, ValueCodec, error::Result};
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
//!     const TREE_NAME: TreeName = TreeName("users");
//!     type Key = u32;
//!     type Value = User;
//! }
//!
//! impl KeyCodec<UserSchema> for u32 {
//!     fn encode_key(&self) -> typed_sled::CodecResult<Vec<u8>> {
//!         Ok(self.to_be_bytes().to_vec())
//!     }
//!     fn decode_key(buf: &[u8]) -> typed_sled::CodecResult<Self> {
//!         if buf.len() != 4 {
//!             return Err(CodecError::InvalidKeyLength {
//!                 schema: UserSchema::TREE_NAME.0,
//!                 expected: 4,
//!                 actual: buf.len(),
//!             });
//!         }
//!         let mut bytes = [0; 4];
//!         bytes.copy_from_slice(buf);
//!         Ok(u32::from_be_bytes(bytes))
//!     }
//! }
//!
//! impl ValueCodec<UserSchema> for User {
//!     fn encode_value(&self) -> typed_sled::CodecResult<Vec<u8>> {
//!         borsh::to_vec(self).map_err(|e| CodecError::SerializationFailed {
//!             schema: UserSchema::TREE_NAME.0,
//!             source: e.into(),
//!         })
//!     }
//!     fn decode_value(buf: &[u8]) -> typed_sled::CodecResult<Self> {
//!         borsh::from_slice(buf).map_err(|e| CodecError::DeserializationFailed {
//!             schema: UserSchema::TREE_NAME.0,
//!             source: e.into(),
//!         })
//!     }
//! }
//!
//! fn main() -> Result<()> {
//!     let sled_db = Arc::new(sled::open("mydb").unwrap());
//!     let db = SledDb::new(sled_db)?;
//!     let tree = db.get_tree::<UserSchema>()?;
//!
//!     let user = User {
//!         id: 1,
//!         name: "Alice".to_string(),
//!     };
//!     tree.insert(&1, &user)?;
//!
//!     let retrieved = tree.get(&1)?;
//!     println!("{:?}", retrieved);
//!
//!     Ok(())
//! }
//! ```

pub mod batch;
pub mod codec;
pub mod db;
pub mod error;
pub mod schema;
pub mod transaction;
pub mod tree;

// Re-export main types
pub use codec::{CodecError, CodecResult, KeyCodec, ValueCodec};
pub use db::SledDb;
pub use schema::{Schema, TreeName};
pub use tree::SledTree;
