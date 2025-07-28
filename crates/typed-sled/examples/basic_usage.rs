use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use typed_sled::{
    CodecError, KeyCodec, Schema, SledDb, SledTree, TreeName, ValueCodec, error::Result,
};

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
struct User {
    id: u32,
    name: String,
    email: String,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
struct Account {
    user_id: u32,
    balance: u64,
}

#[derive(Debug)]
struct UserSchema;

impl KeyCodec<UserSchema> for u32 {
    fn encode_key(&self) -> typed_sled::CodecResult<Vec<u8>> {
        Ok(self.to_be_bytes().to_vec())
    }

    fn decode_key(buf: &[u8]) -> typed_sled::CodecResult<Self> {
        if buf.len() != 1 {
            return Err(CodecError::InvalidLength {
                expected: 1,
                got: buf.len(),
            });
        }
        let buf = [buf[0]];
        Ok(u8::from_be_bytes(buf).into())
    }
}

impl ValueCodec<UserSchema> for User {
    fn encode_value(&self) -> typed_sled::CodecResult<Vec<u8>> {
        borsh::to_vec(self).map_err(CodecError::Serialization)
    }
    fn decode_value(buf: &[u8]) -> typed_sled::CodecResult<Self> {
        borsh::from_slice(buf).map_err(CodecError::Deserialization)
    }
}

impl Schema for UserSchema {
    const TREE_NAME: TreeName = TreeName("users");
    type Key = u32;
    type Value = User;
}

fn main() -> Result<()> {
    // Open the database
    let sled_db = Arc::new(sled::open("example_db").unwrap());
    let db = SledDb::new(sled_db)?;

    // Get typed trees for each schema
    let users: SledTree<UserSchema> = db.get_tree()?;

    // Create some data
    let user = User {
        id: 1,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };

    // Insert data using typed trees
    println!("Inserting user: {user:?}");
    users.put(&user.id, &user)?;

    // Retrieve data
    println!("\nRetrieving user with id 1:");
    if let Some(retrieved_user) = users.get(&1)? {
        println!("Found user: {retrieved_user:?}");
    } else {
        println!("User not found");
    }

    // Try to get non-existent data
    println!("\nTrying to retrieve user with id 999:");
    if let Some(user) = users.get(&999)? {
        println!("Found user: {user:?}");
    } else {
        println!("User not found (as expected)");
    }

    // Remove data
    println!("\nRemoving user 1");
    users.remove(&1)?;

    // Verify removal
    if users.get(&1)?.is_some() {
        println!("User still exists (unexpected)");
    } else {
        println!("User successfully removed");
    }

    println!("\nExample completed successfully!");
    Ok(())
}
