use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use typed_sled::{Schema, SledDb, SledTree, TreeName, error::Result};

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

impl Schema for UserSchema {
    const TREE_NAME: TreeName = TreeName("users");
    type Key = u32;
    type Value = User;
}

#[derive(Debug)]
struct AccountSchema;

impl Schema for AccountSchema {
    const TREE_NAME: TreeName = TreeName("accounts");
    type Key = u32;
    type Value = Account;
}

fn main() -> Result<()> {
    // Open the database
    let sled_db = Arc::new(sled::open("example_db").unwrap());
    let db = SledDb::new(sled_db)?;

    // Get typed trees for each schema
    let users: SledTree<UserSchema> = db.get_tree()?;
    let accounts: SledTree<AccountSchema> = db.get_tree()?;

    // Create some data
    let user = User {
        id: 1,
        name: "Alice".to_string(),
        email: "alice@example.com".to_string(),
    };

    let account = Account {
        user_id: 1,
        balance: 1000,
    };

    // Insert data using typed trees
    println!("Inserting user: {user:?}");
    users.put(&user.id, &user)?;

    println!("Inserting account: {account:?}");
    accounts.put(&account.user_id, &account)?;

    // Retrieve data
    println!("\nRetrieving user with id 1:");
    if let Some(retrieved_user) = users.get(&1)? {
        println!("Found user: {retrieved_user:?}");
    } else {
        println!("User not found");
    }

    println!("\nRetrieving account for user 1:");
    if let Some(retrieved_account) = accounts.get(&1)? {
        println!("Found account: {retrieved_account:?}");
    } else {
        println!("Account not found");
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
