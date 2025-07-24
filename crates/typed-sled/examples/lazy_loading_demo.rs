use std::sync::Arc;

use borsh::{BorshDeserialize, BorshSerialize};
use typed_sled::{Schema, SledDb, SledTree, TreeName, error::Result};

#[derive(BorshSerialize, BorshDeserialize, Debug)]
struct Settings {
    theme: String,
    notifications: bool,
}

#[derive(Debug)]
struct SettingsSchema;

impl Schema for SettingsSchema {
    const TREE_NAME: TreeName = TreeName("settings");
    type Key = String;
    type Value = Settings;
}

fn main() -> Result<()> {
    // Open database with NO pre-declared trees
    let sled_db = Arc::new(sled::open("lazy_demo_db").unwrap());
    let db = SledDb::new(sled_db)?;

    println!("Database opened with no pre-declared trees");

    // Access a tree that wasn't pre-declared - should work due to lazy loading!
    println!("Accessing SettingsSchema tree (not pre-declared)...");
    let settings: SledTree<SettingsSchema> = db.get_tree()?;

    // Insert data
    let my_settings = Settings {
        theme: "dark".to_string(),
        notifications: true,
    };

    println!("Inserting settings: {my_settings:?}");
    settings.put(&"user1".to_string(), &my_settings)?;

    // Retrieve data
    if let Some(retrieved) = settings.get(&"user1".to_string())? {
        println!("Retrieved settings: {retrieved:?}");
    }

    println!("Lazy loading demo completed successfully!");
    Ok(())
}
