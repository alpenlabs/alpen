//! Test to verify strata-merkle MMR proof generation API

#[cfg(test)]
mod tests {
    use strata_acct_types::mmr::{CompactMmr64, Hash, Mmr64};

    #[test]
    fn test_mmr_api_exploration() {
        // Create a new MMR
        let mut mmr = Mmr64::new(16);

        // Add some leaves
        let leaf1: Hash = [1u8; 32];
        let leaf2: Hash = [2u8; 32];
        let leaf3: Hash = [3u8; 32];

        mmr.add_leaf(leaf1).expect("add leaf1");
        mmr.add_leaf(leaf2).expect("add leaf2");
        mmr.add_leaf(leaf3).expect("add leaf3");

        // Check basic properties
        assert_eq!(mmr.num_entries(), 3);

        println!("MMR created with {} leaves", mmr.num_entries());
        println!("Available methods on Mmr64:");

        // Try to generate a proof (this will fail if method doesn't exist)
        // Checking available methods at compile time

        // Convert to compact to see what's available
        let compact: CompactMmr64 = mmr.to_compact();
        println!("Compact MMR has {} peaks", compact.peaks_slice().len());

        // Check if verification works
        // compact.verify::<StrataHasher>(...) - need proof first
    }
}
