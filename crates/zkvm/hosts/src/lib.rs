//! ZKVM hosts for the Alpen codebase.

use strata_primitives::proof::{ProofKey, ProofZkVm};
use zkaleido::{VerifyingKey, ZkVmVerifier};

pub mod native;
#[cfg(feature = "risc0")]
pub mod risc0;
#[cfg(feature = "sp1")]
pub mod sp1;

/// Retrieves the [`VerifyingKey`] for the specified proof key.
///
/// This function determines the appropriate ZkVm host based on the provided [`ProofKey`]
/// and retrieves the corresponding verification key.
///
/// # Panics
///
/// * If the `sp1` feature is not enabled and an SP1 host is requested.
/// * If the `risc0` feature is not enabled and a Risc0 host is requested.
/// * If the host type is unsupported or not recognized.
pub fn get_verification_key(key: &ProofKey) -> VerifyingKey {
    match key.host() {
        ProofZkVm::SP1 => {
            #[cfg(feature = "sp1")]
            {
                sp1::get_host(key.context()).vk()
            }
            #[cfg(not(feature = "sp1"))]
            {
                panic!("The `sp1` feature is not enabled. Enable the feature to use SP1 functionality.");
            }
        }
        ProofZkVm::Risc0 => {
            #[cfg(feature = "risc0")]
            {
                risc0::get_host(key.context()).vk()
            }
            #[cfg(not(feature = "risc0"))]
            {
                panic!("The `risc0` feature is not enabled. Enable the feature to use Risc0 functionality.");
            }
        }
        ProofZkVm::Native => native::get_host(key.context()).vk(),
        _ => panic!("Unsupported ZkVm"),
    }
}

/// Represents a host instance for a ZKVM, wrapping different host implementations that adhere to
/// the [`ZkVmHost`](zkaleido::ZkVmHost) trait.
///
/// This enum provides a type-safe abstraction over various host implementations, such as native,
/// SP1, and Risc0, which each implement the [`ZkVmHost`](zkaleido::ZkVmHost) trait. The
/// [`ZkVmHost`](zkaleido::ZkVmHost) trait is not object-safe, so this enum is used to encapsulate
/// the different implementations.
#[derive(Debug)]
#[non_exhaustive]
pub enum ZkVmHostInstance {
    /// Represents the native ZKVM host implementation.
    ///
    /// This variant uses the [`zkaleido_native_adapter::NativeHost`] implementation
    /// to provide ZKVM functionality without requiring any feature flags.
    Native(zkaleido_native_adapter::NativeHost),

    /// Represents the SP1 ZKVM host implementation.
    ///
    /// This variant uses the [`zkaleido_sp1_host::SP1Host`] implementation and is only
    /// available when the `sp1` feature flag is enabled. Attempting to use this variant
    /// without enabling the `sp1` feature will result in a compile-time error or a runtime panic.
    #[cfg(feature = "sp1")]
    SP1(&'static zkaleido_sp1_host::SP1Host),

    /// Represents the Risc0 ZKVM host implementation.
    ///
    /// This variant uses the [`zkaleido_risc0_host::Risc0Host`] implementation and is only
    /// available when the `risc0` feature flag is enabled. Attempting to use this variant
    /// without enabling the `risc0` feature will result in a compile-time error or a runtime
    /// panic.
    #[cfg(feature = "risc0")]
    Risc0(&'static zkaleido_risc0_host::Risc0Host),
}

/// Resolves the appropriate ZKVM host instance based on the provided [`ProofKey`].
///
/// This function matches the ZKVM type from the [`ProofKey`] and selects the corresponding host
/// implementation. The selected host must be supported and enabled via feature flag:
///
/// ### Panics
/// - If an unsupported ZKVM type is encountered.
pub fn resolve_host(proof_key: &ProofKey) -> ZkVmHostInstance {
    match proof_key.host() {
        ProofZkVm::Native => ZkVmHostInstance::Native(native::get_host(proof_key.context())),
        ProofZkVm::SP1 => {
            #[cfg(feature = "sp1")]
            {
                ZkVmHostInstance::SP1(sp1::get_host(proof_key.context()))
            }
            #[cfg(not(feature = "sp1"))]
            {
                panic!(
                    "The `sp1` feature is not enabled. Enable the feature to use SP1 functionality"
                );
            }
        }
        ProofZkVm::Risc0 => {
            #[cfg(feature = "risc0")]
            {
                ZkVmHostInstance::Risc0(risc0::get_host(proof_key.context()))
            }
            #[cfg(not(feature = "risc0"))]
            {
                panic!("The `risc0` feature is not enabled. Enable the feature to use Risc0 functionality");
            }
        }
        _ => panic!("Unsupported ZkVm"),
    }
}
