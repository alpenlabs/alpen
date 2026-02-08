//! Generic Arbitrary generator for the Alpen codebase.

use arbitrary::{Arbitrary, Unstructured};
use rand_core::{CryptoRngCore, OsRng};

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
    pub fn generate<T>(&mut self) -> T
    where
        T: for<'a> Arbitrary<'a> + Clone,
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
    pub fn generate_with_rng<T, R>(&mut self, rng: &mut R) -> T
    where
        T: for<'a> Arbitrary<'a> + Clone,
        R: CryptoRngCore,
    {
        const MAX_ATTEMPTS: usize = 16;
        let mut last_error = None;

        for _ in 0..MAX_ATTEMPTS {
            rng.fill_bytes(&mut self.buf);
            let mut u = Unstructured::new(&self.buf);
            match T::arbitrary(&mut u) {
                Ok(value) => return value,
                Err(err) => last_error = Some(err),
            }
        }

        let error_msg = last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "unknown error".to_string());
        panic!("Failed to generate arbitrary instance: {error_msg}");
    }
}
