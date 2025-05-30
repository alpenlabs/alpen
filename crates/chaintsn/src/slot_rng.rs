//! Deterministic cryptographically-secure PRNG for use when picking stuff in the CL STF.
//!
//! This uses `ChaCha12Rng` from the `rand_chacha` crate.

use rand_chacha::ChaCha12Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};

/// Deterministic CSPRNG used within the scope of a block's slot processing.
/// WARNING: This is _not_ suitable for use cases like key generation!
#[derive(Debug)]
pub struct SlotRng {
    rng: rand_chacha::ChaCha12Rng,
}

impl CryptoRng for SlotRng {}

impl SeedableRng for SlotRng {
    type Seed = [u8; 32];

    fn from_seed(seed: Self::Seed) -> Self {
        Self {
            rng: ChaCha12Rng::from_seed(seed),
        }
    }
}

impl RngCore for SlotRng {
    fn next_u32(&mut self) -> u32 {
        self.rng.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.rng.fill_bytes(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.rng.try_fill_bytes(dest)
    }
}

impl SlotRng {
    /// Returns a randomly-generated array of bytes.
    pub fn next_arr<const N: usize>(&mut self) -> [u8; N] {
        let mut arr = [0u8; N];
        self.fill_bytes(&mut arr);

        arr
    }

    /// Returns a pseudorandom `u8`.
    pub fn next_u8(&mut self) -> u8 {
        let byte = self.next_u32();
        (byte & 0xff) as u8
    }
}

#[cfg(test)]
mod test {
    use bitcoin::key::rand::Rng;

    use super::*;

    #[test]
    // Identical seeds should yield identical outputs
    fn test_identical() {
        assert_eq!(
            SlotRng::from_seed([0u8; 32]).next_u32(),
            SlotRng::from_seed([0u8; 32]).next_u32()
        );
    }

    #[test]
    // Distinct seeds should yield distinct outputs
    fn test_unique() {
        assert_ne!(
            SlotRng::from_seed([0u8; 32]).next_u32(),
            SlotRng::from_seed([1u8; 32]).next_u32()
        );
    }

    #[test]
    // Arrays should be filled with data correctly
    fn test_array() {
        // Generate and return a random array
        let next_arr = SlotRng::from_seed([1u8; 32]).next_arr::<32>();

        // Generate the same array using `RngCore` functionality
        let mut rng = SlotRng::from_seed([1u8; 32]);
        let mut fill_arr = [0u8; 32];
        rng.fill(&mut fill_arr);

        // They should match and contain fresh data
        assert_eq!(next_arr, fill_arr);
        assert_ne!(fill_arr, [0u8; 32]);
    }

    #[test]
    // Generation of a `u8` should be correct
    fn test_u8() {
        // Generate a `u8` on its own
        let standalone_u8 = SlotRng::from_seed([1u8; 32]).next_u8();

        // Generate a `u8` from an array
        let arr_u8 = SlotRng::from_seed([1u8; 32]).next_arr::<1>()[0];

        assert_eq!(standalone_u8, arr_u8);
    }

    #[test]
    // Generation of `u32` and `u64` should be bitwise consistent
    fn test_u32_u64() {
        // Generate a `u32` on its own
        let standalone_u32 = SlotRng::from_seed([2u8; 32]).next_u32();

        // Generate a `u32` by masking a `u64`
        let masked_u32 = (SlotRng::from_seed([2u8; 32]).next_u64() & 0xFFFF_FFFFu64) as u32;

        assert_eq!(standalone_u32, masked_u32);
    }
}
