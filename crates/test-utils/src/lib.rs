//! Test utilities for the Alpen codebase.

use arbitrary::{Arbitrary, Unstructured};
use rand_core::{CryptoRngCore, OsRng};

pub mod bitcoin;
pub mod bitcoin_mainnet_segment;
pub mod bridge;
pub mod evm_ee;
pub mod l2;
pub mod tx_indexer;

/// The default buffer size for the `ArbitraryGenerator`.
const ARB_GEN_LEN: usize = 65_536;

#[derive(Debug)]
pub struct ArbitraryGenerator {
    buf: Vec<u8>, // Persistent buffer
}

impl Default for ArbitraryGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl ArbitraryGenerator {
    /// Creates a new `ArbitraryGenerator` with a default buffer size.
    ///
    /// # Returns
    ///
    /// A new instance of `ArbitraryGenerator`.
    pub fn new() -> Self {
        Self::new_with_size(ARB_GEN_LEN)
    }

    /// Creates a new `ArbitraryGenerator` with a specified buffer size.
    ///
    /// # Arguments
    ///
    /// * `s` - The size of the buffer to be used.
    ///
    /// # Returns
    ///
    /// A new instance of `ArbitraryGenerator` with the specified buffer size.
    pub fn new_with_size(s: usize) -> Self {
        Self { buf: vec![0u8; s] }
    }

    /// Generates an arbitrary instance of type `T` using the default RNG, [`OsRng`].
    ///
    /// # Returns
    ///
    /// An arbitrary instance of type `T`.
    pub fn generate<'a, T>(&'a mut self) -> T
    where
        T: Arbitrary<'a> + Clone,
    {
        self.generate_with_rng::<T, OsRng>(&mut OsRng)
    }

    /// Generates an arbitrary instance of type `T`.
    ///
    /// # Arguments
    ///
    /// * `rng` - An RNG to be used for generating the arbitrary instance. Provided RNG must
    ///   implement the [`CryptoRngCore`] trait.
    ///
    /// # Returns
    ///
    /// An arbitrary instance of type `T`.
    pub fn generate_with_rng<'a, T, R>(&'a mut self, rng: &mut R) -> T
    where
        T: Arbitrary<'a> + Clone,
        R: CryptoRngCore,
    {
        rng.fill_bytes(&mut self.buf);
        let mut u = Unstructured::new(&self.buf);
        T::arbitrary(&mut u).expect("Failed to generate arbitrary instance")
    }
}
